//! Importers command - Find all files that import a given module
//!
//! Returns an ImportersReport with module name, list of importing files, and total count.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use colored::Colorize;

use tldr_core::types::ImportersReport;
use tldr_core::{find_importers, Language};

use crate::commands::daemon_router::{params_with_module, try_daemon_route};
use crate::output::{format_importers_text, OutputFormat, OutputWriter};

/// Find all files that import a given module
#[derive(Debug, Args)]
pub struct ImportersArgs {
    /// Module name to search for
    pub module: String,

    /// Directory to search (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Programming language (auto-detected from directory if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Maximum number of importing files to show (0 = unlimited)
    #[arg(long, short = 'm', default_value = "50")]
    pub limit: usize,
}

impl ImportersArgs {
    /// Run the importers command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Try daemon first for cached result
        if let Some(mut result) = try_daemon_route::<ImportersReport>(
            &self.path,
            "importers",
            params_with_module(&self.module, Some(&self.path)),
        ) {
            self.apply_limit(&mut result);
            self.output_result(&writer, &result)?;
            return Ok(());
        }

        // Determine language (auto-detect from directory, default to Python)
        let language = self
            .lang
            .unwrap_or_else(|| Language::from_directory(&self.path).unwrap_or(Language::Python));

        // Fallback to direct compute
        writer.progress(&format!(
            "Finding files that import '{}' in {} ({:?})...",
            self.module,
            self.path.display(),
            language
        ));

        // Find importers
        let mut result = find_importers(&self.path, &self.module, language)?;
        self.apply_limit(&mut result);
        self.output_result(&writer, &result)?;

        Ok(())
    }

    fn apply_limit(&self, report: &mut ImportersReport) {
        if self.limit > 0 && report.importers.len() > self.limit {
            report.importers.truncate(self.limit);
        }
    }

    fn output_result(&self, writer: &OutputWriter, report: &ImportersReport) -> Result<()> {
        if writer.is_text() {
            let shown = report.importers.len();
            let total = report.total;
            let truncated = shown < total;

            let header = if truncated {
                format!(
                    "{} imported by {} files (showing {})\n",
                    format!("\"{}\"", report.module).bold(),
                    total,
                    shown,
                )
            } else {
                format!(
                    "{} imported by {} {}\n",
                    format!("\"{}\"", report.module).bold(),
                    total,
                    if total == 1 { "file" } else { "files" },
                )
            };

            writer.write_text(&format!("{}\n{}", header, format_importers_text(report)))?;
        } else {
            writer.write(report)?;
        }
        Ok(())
    }
}
