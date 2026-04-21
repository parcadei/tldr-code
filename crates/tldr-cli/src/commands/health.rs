//! Health command - Comprehensive code health dashboard
//!
//! Aggregates multiple sub-analyzers into a unified health report:
//! - Complexity analysis (cyclomatic complexity, hotspots)
//! - Cohesion analysis (LCOM4 class cohesion)
//! - Dead code detection (unreachable functions)
//! - Martin metrics (package coupling: Ca, Ce, I, A, D)
//! - Coupling analysis (pairwise module coupling, full mode)
//! - Similarity analysis (function clone detection, full mode)
//!
//! # Premortem Mitigations
//! - T20: value_parser for --detail validation
//! - T21: All health errors map to exit code 2
//! - T23: Validate --quick + --detail=coupling/similar conflict

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, ValueEnum};

use tldr_core::quality::health::{run_health, HealthOptions, HealthReport};
use tldr_core::quality::ThresholdPreset;
use tldr_core::Language;

use crate::output::{OutputFormat, OutputWriter};

/// Comprehensive code health analysis
#[derive(Debug, Args)]
pub struct HealthArgs {
    /// Path to analyze (file or directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Show detailed sub-analyzer output
    ///
    /// Valid values: complexity, cohesion, dead_code, martin, coupling, similarity, all
    #[arg(long, value_parser = detail_parser)]
    pub detail: Option<String>,

    /// Quick mode (skip coupling and similarity - faster)
    #[arg(long)]
    pub quick: bool,

    /// Threshold preset (strict, default, relaxed)
    #[arg(long, value_enum, default_value = "default")]
    pub preset: PresetArg,

    /// Maximum items to return for coupling and similarity analyses (default: 50)
    #[arg(long, default_value = "50")]
    pub max_items: usize,

    /// Summary mode - omit detail arrays, only include summary metrics
    #[arg(long)]
    pub summary: bool,
}

/// Threshold preset for CLI (mirrors ThresholdPreset)
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum PresetArg {
    /// Strict thresholds for high-quality codebases
    Strict,
    /// Default thresholds (recommended)
    #[default]
    Default,
    /// Relaxed thresholds for legacy code
    Relaxed,
}

impl From<PresetArg> for ThresholdPreset {
    fn from(arg: PresetArg) -> Self {
        match arg {
            PresetArg::Strict => ThresholdPreset::Strict,
            PresetArg::Default => ThresholdPreset::Default,
            PresetArg::Relaxed => ThresholdPreset::Relaxed,
        }
    }
}

/// T20 Mitigation: value_parser for --detail flag
fn detail_parser(s: &str) -> Result<String, String> {
    let valid = [
        "complexity",
        "cohesion",
        "dead_code",
        "martin",
        "coupling",
        "similarity",
        "all",
    ];
    if valid.contains(&s) {
        Ok(s.to_string())
    } else {
        Err(format!(
            "Invalid detail value '{}'. Valid values: {}",
            s,
            valid.join(", ")
        ))
    }
}

impl HealthArgs {
    /// Validate CLI arguments (T23: check --quick + --detail conflict)
    fn validate(&self) -> Result<()> {
        // T23 Mitigation: --quick + --detail=coupling/similar conflict
        if self.quick {
            if let Some(ref detail) = self.detail {
                if detail == "coupling" || detail == "similarity" {
                    anyhow::bail!(
                        "--detail={} requires full mode. Remove --quick flag to analyze {}.",
                        detail,
                        detail
                    );
                }
            }
        }
        Ok(())
    }

    /// Run the health command
    ///
    /// `lang` is passed from the global CLI `--lang` / `-l` flag (already parsed as `Language` enum).
    pub fn run(&self, format: OutputFormat, quiet: bool, lang: Option<Language>) -> Result<()> {
        // Validate arguments first (T23)
        self.validate()?;

        let writer = OutputWriter::new(format, quiet);

        // Validate path exists
        if !self.path.exists() {
            anyhow::bail!("Path not found: {}", self.path.display());
        }

        writer.progress(&format!(
            "Analyzing code health in {}{}...",
            self.path.display(),
            if self.quick { " (quick mode)" } else { "" }
        ));

        // Language comes from global CLI flag (already parsed)
        let language = lang;

        // Build options
        let mut options = HealthOptions {
            quick: self.quick,
            preset: self.preset.into(),
            max_items: self.max_items,
            summary: self.summary,
            ..HealthOptions::with_preset(self.preset.into())
        };
        options.max_items = self.max_items;
        options.summary = self.summary;

        // Run health analysis
        let report = run_health(&self.path, language, options)?;

        // Output based on format, --detail flag, and --summary flag
        if self.summary && self.detail.is_none() {
            // Summary mode: only output summary metrics
            output_summary(&writer, &report, format)?;
        } else {
            output_report(&writer, &report, format, self.detail.as_deref())?;
        }

        Ok(())
    }
}

/// Parse language string to Language enum (T27: error with suggestions)
#[allow(dead_code)]
fn parse_language(lang: &str) -> Result<Language> {
    match lang.to_lowercase().as_str() {
        "python" | "py" => Ok(Language::Python),
        "typescript" | "ts" => Ok(Language::TypeScript),
        "javascript" | "js" => Ok(Language::JavaScript),
        "rust" | "rs" => Ok(Language::Rust),
        "go" => Ok(Language::Go),
        "java" => Ok(Language::Java),
        "c" => Ok(Language::C),
        "cpp" | "c++" => Ok(Language::Cpp),
        "ruby" | "rb" => Ok(Language::Ruby),
        "php" => Ok(Language::Php),
        "swift" => Ok(Language::Swift),
        "kotlin" | "kt" => Ok(Language::Kotlin),
        "scala" => Ok(Language::Scala),
        "csharp" | "cs" | "c#" => Ok(Language::CSharp),
        "lua" => Ok(Language::Lua),
        "luau" => Ok(Language::Luau),
        "elixir" | "ex" => Ok(Language::Elixir),
        "ocaml" | "ml" => Ok(Language::Ocaml),
        _ => anyhow::bail!(
            "Unsupported language: '{}'. Supported: python, typescript, javascript, rust, go, java, c, cpp, ruby, php, swift, kotlin, scala, csharp, lua, luau, elixir, ocaml",
            lang
        ),
    }
}

/// Output the health report based on format and detail flag
fn output_report(
    writer: &OutputWriter,
    report: &HealthReport,
    _format: OutputFormat,
    detail: Option<&str>,
) -> Result<()> {
    match detail {
        Some("all") => {
            // Output entire report
            if writer.is_text() {
                writer.write_text(&report.to_text())?;
            } else {
                writer.write(report)?;
            }
        }
        Some(sub_name) => {
            // Output only the specified sub-analysis
            if let Some(details) = report.detail(sub_name) {
                if writer.is_text() {
                    // For text, show a formatted version of the sub-analysis
                    let text = format!(
                        "{} Analysis\n{}\n{}",
                        sub_name,
                        "=".repeat(40),
                        serde_json::to_string_pretty(details).unwrap_or_default()
                    );
                    writer.write_text(&text)?;
                } else {
                    writer.write(details)?;
                }
            } else if let Some(result) = report.sub_results.get(sub_name) {
                // Sub-analysis exists but has no details (e.g., failed or skipped)
                if writer.is_text() {
                    let msg = result.error.as_deref().unwrap_or("No details available");
                    writer.write_text(&format!("{}: {}", sub_name, msg))?;
                } else {
                    writer.write(result)?;
                }
            } else {
                anyhow::bail!(
                    "Sub-analysis '{}' not found. Available: {}",
                    sub_name,
                    report
                        .sub_results
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        None => {
            // Default: output full report
            if writer.is_text() {
                writer.write_text(&report.to_text())?;
            } else {
                writer.write(report)?;
            }
        }
    }
    Ok(())
}

/// Output summary-only mode (omits detail arrays)
fn output_summary(
    writer: &OutputWriter,
    report: &HealthReport,
    _format: OutputFormat,
) -> Result<()> {
    // Create a summary-only output structure
    let summary_output = serde_json::json!({
        "wrapper": "health",
        "path": report.path.display().to_string(),
        "language": report.language,
        "quick_mode": report.quick_mode,
        "total_elapsed_ms": report.total_elapsed_ms,
        "summary": report.summary,
        "errors": report.errors,
    });

    if writer.is_text() {
        writer.write_text(&report.to_text())?;
    } else {
        writer.write(&summary_output)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detail_parser_valid() {
        assert!(detail_parser("complexity").is_ok());
        assert!(detail_parser("cohesion").is_ok());
        assert!(detail_parser("dead_code").is_ok());
        assert!(detail_parser("martin").is_ok());
        assert!(detail_parser("coupling").is_ok());
        assert!(detail_parser("similarity").is_ok());
        assert!(detail_parser("all").is_ok());
    }

    #[test]
    fn test_detail_parser_invalid() {
        let result = detail_parser("invalid");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Invalid detail value"));
        assert!(err.contains("complexity"));
    }

    #[test]
    fn test_parse_language_valid() {
        assert!(matches!(parse_language("python"), Ok(Language::Python)));
        assert!(matches!(parse_language("py"), Ok(Language::Python)));
        assert!(matches!(parse_language("Python"), Ok(Language::Python)));
        assert!(matches!(
            parse_language("typescript"),
            Ok(Language::TypeScript)
        ));
        assert!(matches!(parse_language("ts"), Ok(Language::TypeScript)));
        assert!(matches!(parse_language("rust"), Ok(Language::Rust)));
        assert!(matches!(parse_language("go"), Ok(Language::Go)));
    }

    #[test]
    fn test_parse_language_invalid() {
        let result = parse_language("unknown");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unsupported language"));
        assert!(err.contains("python"));
    }

    #[test]
    fn test_validate_quick_coupling_conflict() {
        let args = HealthArgs {
            path: PathBuf::from("."),
            detail: Some("coupling".to_string()),
            quick: true,
            preset: PresetArg::Default,
            max_items: 50,
            summary: false,
        };
        let result = args.validate();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("--detail=coupling requires full mode"));
    }

    #[test]
    fn test_validate_quick_similarity_conflict() {
        let args = HealthArgs {
            path: PathBuf::from("."),
            detail: Some("similarity".to_string()),
            quick: true,
            preset: PresetArg::Default,
            max_items: 50,
            summary: false,
        };
        let result = args.validate();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("--detail=similarity requires full mode"));
    }

    #[test]
    fn test_validate_quick_complexity_ok() {
        let args = HealthArgs {
            path: PathBuf::from("."),
            detail: Some("complexity".to_string()),
            quick: true,
            preset: PresetArg::Default,
            max_items: 50,
            summary: false,
        };
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_preset_conversion() {
        assert!(matches!(
            ThresholdPreset::from(PresetArg::Strict),
            ThresholdPreset::Strict
        ));
        assert!(matches!(
            ThresholdPreset::from(PresetArg::Default),
            ThresholdPreset::Default
        ));
        assert!(matches!(
            ThresholdPreset::from(PresetArg::Relaxed),
            ThresholdPreset::Relaxed
        ));
    }
}
