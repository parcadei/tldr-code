//! Tree command - Show file tree
//!
//! Displays the file tree structure of a directory.
//! Auto-routes through daemon when available for ~35x speedup.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::types::FileTree;
use tldr_core::{get_file_tree, IgnoreSpec};

use crate::commands::daemon_router::{params_with_path, try_daemon_route};
use crate::output::{format_file_tree_text, OutputFormat, OutputWriter};

/// Show file tree structure
#[derive(Debug, Args)]
pub struct TreeArgs {
    /// Directory to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Filter by file extensions (e.g., --ext .py --ext .rs)
    #[arg(long = "ext", short = 'e')]
    pub extensions: Vec<String>,

    /// Include hidden files and directories
    #[arg(long, short = 'H')]
    pub include_hidden: bool,
}

impl TreeArgs {
    /// Run the tree command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Try daemon first for cached result
        if let Some(tree) =
            try_daemon_route::<FileTree>(&self.path, "tree", params_with_path(Some(&self.path)))
        {
            // Output based on format
            if writer.is_text() {
                let text = format_file_tree_text(&tree, 0);
                writer.write_text(&text)?;
                return Ok(());
            } else {
                writer.write(&tree)?;
                return Ok(());
            }
        }

        // Fallback to direct compute

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

        // Get file tree
        let tree = get_file_tree(
            &self.path,
            extensions.as_ref(),
            !self.include_hidden,
            Some(&IgnoreSpec::default()),
        )?;

        // Output based on format
        if writer.is_text() {
            let text = format_file_tree_text(&tree, 0);
            writer.write_text(&text)?;
        } else {
            writer.write(&tree)?;
        }

        Ok(())
    }
}
