//! Debt command - Technical debt analysis using SQALE method
//!
//! Analyzes source code to estimate technical debt using the SQALE
//! (Software Quality Assessment based on Lifecycle Expectations) method.
//! Each issue is assigned a remediation time in minutes, aggregated into
//! summary statistics including debt ratio and density.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::quality::debt::{analyze_debt, DebtOptions, DebtReport};
use tldr_core::Language;

use crate::output::{OutputFormat, OutputWriter};

/// Valid SQALE categories for filtering
const VALID_CATEGORIES: [&str; 6] = [
    "reliability",
    "security",
    "maintainability",
    "efficiency",
    "changeability",
    "testability",
];

/// Analyze technical debt using SQALE method
#[derive(Debug, Args)]
pub struct DebtArgs {
    /// Path to analyze (file or directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Filter by SQALE category
    #[arg(short = 'c', long, value_parser = ["reliability", "security", "maintainability", "efficiency", "changeability", "testability"])]
    pub category: Option<String>,

    /// Number of top files to show
    #[arg(short = 'k', long, default_value = "20")]
    pub top: usize,

    /// Minimum debt minutes to include file
    #[arg(long)]
    pub min_debt: Option<u32>,

    /// Hourly rate for cost estimation ($/hour)
    #[arg(long)]
    pub hourly_rate: Option<f64>,
}

impl DebtArgs {
    /// Run the debt command
    ///
    /// `lang` is passed from the global CLI `--lang` / `-l` flag (already parsed as `Language` enum).
    pub fn run(&self, format: OutputFormat, quiet: bool, lang: Option<Language>) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Validate path exists (PM-5: exit code 1 for user errors)
        if !self.path.exists() {
            anyhow::bail!("Path not found: {}", self.path.display());
        }

        // Validate category if provided (PM-4: validate before analysis)
        if let Some(ref cat) = self.category {
            if !VALID_CATEGORIES.contains(&cat.as_str()) {
                anyhow::bail!(
                    "Invalid category '{}'. Valid categories: {}",
                    cat,
                    VALID_CATEGORIES.join(", ")
                );
            }
        }

        writer.progress(&format!(
            "Analyzing technical debt in {}...",
            self.path.display()
        ));

        // Language comes from global CLI flag (already parsed)
        let language = lang;

        let options = DebtOptions {
            path: self.path.clone(),
            category_filter: self.category.clone(),
            language,
            top_k: self.top,
            min_debt: self.min_debt.unwrap_or(0),
            hourly_rate: self.hourly_rate,
        };

        let report = analyze_debt(options)?;

        // Output based on format
        if writer.is_text() {
            let text = report.to_text();
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

/// Parse language string to Language enum
#[allow(dead_code)]
fn parse_language(lang: &str) -> Option<Language> {
    match lang.to_lowercase().as_str() {
        "python" | "py" => Some(Language::Python),
        "typescript" | "ts" => Some(Language::TypeScript),
        "javascript" | "js" => Some(Language::JavaScript),
        "rust" | "rs" => Some(Language::Rust),
        "go" => Some(Language::Go),
        "java" => Some(Language::Java),
        "c" => Some(Language::C),
        "cpp" | "c++" => Some(Language::Cpp),
        "ruby" | "rb" => Some(Language::Ruby),
        "php" => Some(Language::Php),
        "swift" => Some(Language::Swift),
        "kotlin" | "kt" => Some(Language::Kotlin),
        "scala" => Some(Language::Scala),
        "csharp" | "cs" | "c#" => Some(Language::CSharp),
        "lua" => Some(Language::Lua),
        "luau" => Some(Language::Luau),
        "elixir" | "ex" => Some(Language::Elixir),
        "ocaml" | "ml" => Some(Language::Ocaml),
        _ => None,
    }
}

/// Format debt report for text output (delegated to DebtReport::to_text())
#[allow(dead_code)]
fn format_debt_text(report: &DebtReport) -> String {
    report.to_text()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_language_python() {
        assert_eq!(parse_language("python"), Some(Language::Python));
        assert_eq!(parse_language("py"), Some(Language::Python));
        assert_eq!(parse_language("Python"), Some(Language::Python));
    }

    #[test]
    fn test_parse_language_typescript() {
        assert_eq!(parse_language("typescript"), Some(Language::TypeScript));
        assert_eq!(parse_language("ts"), Some(Language::TypeScript));
    }

    #[test]
    fn test_parse_language_unknown() {
        assert_eq!(parse_language("unknown"), None);
        assert_eq!(parse_language(""), None);
    }

    #[test]
    fn test_valid_categories() {
        assert!(VALID_CATEGORIES.contains(&"reliability"));
        assert!(VALID_CATEGORIES.contains(&"maintainability"));
        assert!(!VALID_CATEGORIES.contains(&"invalid"));
    }
}
