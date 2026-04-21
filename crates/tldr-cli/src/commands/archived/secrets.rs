//! Secrets command - Scan for hardcoded secrets
//!
//! Detects potential secrets like API keys, passwords, private keys.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::{scan_secrets, SecretsReport, Severity};

use crate::commands::daemon_router::{params_with_path, try_daemon_route};
use crate::output::{format_secrets_text, OutputFormat, OutputWriter};

/// Scan for hardcoded secrets
#[derive(Debug, Args)]
pub struct SecretsArgs {
    /// Path to scan (file or directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Entropy threshold for high-entropy string detection
    #[arg(long, short = 'e', default_value = "4.5")]
    pub entropy_threshold: f64,

    /// Include test files in scan
    #[arg(long)]
    pub include_test: bool,

    /// Filter by minimum severity
    #[arg(long, short = 's')]
    pub min_severity: Option<SeverityArg>,
}

/// CLI wrapper for severity
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum SeverityArg {
    /// Low severity
    Low,
    /// Medium severity
    Medium,
    /// High severity
    High,
    /// Critical severity
    Critical,
}

impl From<SeverityArg> for Severity {
    fn from(arg: SeverityArg) -> Self {
        match arg {
            SeverityArg::Low => Severity::Low,
            SeverityArg::Medium => Severity::Medium,
            SeverityArg::High => Severity::High,
            SeverityArg::Critical => Severity::Critical,
        }
    }
}

impl SecretsArgs {
    /// Run the secrets command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Try daemon first for cached result
        if let Some(report) = try_daemon_route::<SecretsReport>(
            &self.path,
            "secrets",
            params_with_path(Some(&self.path)),
        ) {
            // Output based on format
            if writer.is_text() {
                let text = format_secrets_text(&report);
                writer.write_text(&text)?;
                return Ok(());
            } else {
                writer.write(&report)?;
                return Ok(());
            }
        }

        // Fallback to direct compute
        writer.progress(&format!(
            "Scanning for secrets in {}...",
            self.path.display()
        ));

        // Scan for secrets
        let report = scan_secrets(
            &self.path,
            self.entropy_threshold,
            self.include_test,
            self.min_severity.map(|s| s.into()),
        )?;

        // Output based on format
        if writer.is_text() {
            let text = format_secrets_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}
