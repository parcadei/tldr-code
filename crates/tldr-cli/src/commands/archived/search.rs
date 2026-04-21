//! Search command - Text search
//!
//! Searches files for regex patterns with context.
//! Auto-routes through daemon when available for ~35x speedup.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::{search, IgnoreSpec, SearchMatch};

use crate::commands::daemon_router::{params_with_pattern, try_daemon_route};
use crate::output::{format_search_text, OutputFormat, OutputWriter};

/// Search files for regex pattern
#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Regex pattern to search for
    pub pattern: String,

    /// Directory to search in (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Filter by file extensions (e.g., --ext .py --ext .rs)
    #[arg(long = "ext", short = 'e')]
    pub extensions: Vec<String>,

    /// Number of context lines before and after each match
    #[arg(long, short = 'C', default_value = "0")]
    pub context: usize,

    /// Maximum number of matches to return
    #[arg(long, short = 'm', default_value = "100")]
    pub max_results: usize,

    /// Maximum number of files to search
    #[arg(long, default_value = "1000")]
    pub max_files: usize,
}

impl SearchArgs {
    /// Run the search command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Try daemon first for cached result
        if let Some(matches) = try_daemon_route::<Vec<SearchMatch>>(
            &self.path,
            "search",
            params_with_pattern(&self.pattern, Some(self.max_results)),
        ) {
            // Output based on format
            if writer.is_text() {
                let text = format_search_text(&matches);
                writer.write_text(&text)?;
                return Ok(());
            } else {
                writer.write(&matches)?;
                return Ok(());
            }
        }

        // Fallback to direct compute
        writer.progress(&format!(
            "Searching for '{}' in {}...",
            self.pattern,
            self.path.display()
        ));

        // Build extensions set if provided
        let extensions: Option<HashSet<String>> = if self.extensions.is_empty() {
            None
        } else {
            Some(
                self.extensions
                    .iter()
                    .map(|s| {
                        if s.starts_with('.') {
                            s.clone()
                        } else {
                            format!(".{}", s)
                        }
                    })
                    .collect(),
            )
        };

        // Search
        let matches = search(
            &self.pattern,
            &self.path,
            extensions.as_ref(),
            self.context,
            self.max_results,
            self.max_files,
            Some(&IgnoreSpec::default()),
        )?;

        // Output based on format
        if writer.is_text() {
            let text = format_search_text(&matches);
            writer.write_text(&text)?;
        } else {
            writer.write(&matches)?;
        }

        Ok(())
    }
}
