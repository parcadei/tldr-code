//! Fix command -- diagnose and auto-fix errors from compiler/runtime output.
//!
//! # Subcommands
//!
//! - `tldr fix diagnose` -- parse error text and produce a structured diagnosis
//! - `tldr fix apply` -- apply a fix to source code, writing the patched output
//! - `tldr fix check` -- run test command, diagnose failures, apply fixes in a loop
//!
//! # Examples
//!
//! ```sh
//! # Pipe a Python traceback from clipboard or file
//! tldr fix diagnose --error-file traceback.txt --source app.py
//!
//! # Inline error text
//! tldr fix diagnose --error "NameError: name 'json' is not defined" --source app.py
//!
//! # Apply fix to source (writes to stdout or --output)
//! tldr fix apply --error-file traceback.txt --source app.py
//!
//! # Run test, diagnose, fix, repeat loop
//! tldr fix check --file src/app.py --test-cmd "pytest tests/test_app.py"
//! ```

use std::io::Read;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};

use tldr_core::fix;
use tldr_core::Language;

use crate::output::{OutputFormat, OutputWriter};

/// Diagnose and auto-fix errors from compiler/runtime output
#[derive(Debug, Args)]
pub struct FixArgs {
    /// Fix subcommand
    #[command(subcommand)]
    pub command: FixCommand,
}

/// Fix subcommands
#[derive(Debug, Subcommand)]
pub enum FixCommand {
    /// Parse error output and produce a structured diagnosis with optional fix
    Diagnose(FixDiagnoseArgs),
    /// Apply fix edits to source code and write the patched result
    Apply(FixApplyArgs),
    /// Run test command, diagnose failures, apply fixes, and re-run in a loop
    Check(FixCheckArgs),
}

/// Arguments for `tldr fix check`
#[derive(Debug, Args)]
pub struct FixCheckArgs {
    /// Source file to fix
    #[arg(long, short = 'f')]
    pub file: PathBuf,

    /// Test command to run (e.g., "pytest tests/test_app.py")
    #[arg(long, short = 't')]
    pub test_cmd: String,

    /// Maximum number of fix attempts (default: 5)
    #[arg(long, default_value = "5")]
    pub max_attempts: usize,
}

/// Arguments for `tldr fix diagnose`
#[derive(Debug, Args)]
pub struct FixDiagnoseArgs {
    /// Source file to analyze (required for tree-sitter based analysis)
    #[arg(long, short = 's')]
    pub source: PathBuf,

    /// Inline error text (mutually exclusive with --error-file)
    #[arg(long, short = 'e', conflicts_with = "error_file")]
    pub error: Option<String>,

    /// File containing error text (mutually exclusive with --error)
    #[arg(long, conflicts_with = "error")]
    pub error_file: Option<PathBuf>,

    /// Read error text from stdin (when neither --error nor --error-file is given)
    #[arg(long)]
    pub stdin: bool,

    /// Path to API surface JSON file for enhanced analysis (e.g., TS2339 property suggestions)
    #[arg(long)]
    pub api_surface: Option<PathBuf>,
}

/// Arguments for `tldr fix apply`
#[derive(Debug, Args)]
pub struct FixApplyArgs {
    /// Source file to patch
    #[arg(long, short = 's')]
    pub source: PathBuf,

    /// Inline error text
    #[arg(long, short = 'e', conflicts_with = "error_file")]
    pub error: Option<String>,

    /// File containing error text
    #[arg(long, conflicts_with = "error")]
    pub error_file: Option<PathBuf>,

    /// Output file for the patched source (stdout if not specified)
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,

    /// Read error text from stdin
    #[arg(long)]
    pub stdin: bool,

    /// Write the patched source back to the original file (in-place fix)
    #[arg(long, short = 'i')]
    pub in_place: bool,

    /// Show a unified diff instead of the full patched source
    #[arg(long, short = 'd')]
    pub diff: bool,

    /// Path to API surface JSON file for enhanced analysis (e.g., TS2339 property suggestions)
    #[arg(long)]
    pub api_surface: Option<PathBuf>,
}

impl FixArgs {
    /// Run the fix command
    pub fn run(&self, format: OutputFormat, _quiet: bool, lang: Option<Language>) -> Result<()> {
        let lang_str = lang.as_ref().map(Language::as_str);
        match &self.command {
            FixCommand::Diagnose(args) => run_diagnose(args, format, lang_str),
            FixCommand::Apply(args) => run_apply(args, format, lang_str),
            FixCommand::Check(args) => run_check(args, format, lang_str),
        }
    }
}

/// Read error text from one of: --error, --error-file, --stdin, or fallback to stdin
fn read_error_text(
    error: &Option<String>,
    error_file: &Option<PathBuf>,
    use_stdin: bool,
) -> Result<String> {
    if let Some(text) = error {
        return Ok(text.clone());
    }

    if let Some(path) = error_file {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("Failed to read error file '{}': {}", path.display(), e))?;
        return Ok(text);
    }

    if use_stdin || (error.is_none() && error_file.is_none()) {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| anyhow!("Failed to read from stdin: {}", e))?;
        if buf.is_empty() {
            return Err(anyhow!(
                "No error text provided. Use --error, --error-file, or pipe to stdin."
            ));
        }
        return Ok(buf);
    }

    Err(anyhow!(
        "No error text provided. Use --error, --error-file, --stdin, or pipe to stdin."
    ))
}

/// Compute a minimal unified diff between two strings, line by line.
///
/// Returns a string with lines prefixed by ` ` (unchanged), `-` (removed), or `+` (added).
fn compute_line_diff(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut output = String::new();

    // Walk both sides; emit context / remove / add markers.
    let mut oi = 0;
    let mut ni = 0;
    while oi < old_lines.len() || ni < new_lines.len() {
        if oi < old_lines.len() && ni < new_lines.len() {
            if old_lines[oi] == new_lines[ni] {
                output.push_str(&format!(" {}\n", old_lines[oi]));
                oi += 1;
                ni += 1;
            } else {
                // Lines differ: emit removal then addition
                output.push_str(&format!("-{}\n", old_lines[oi]));
                output.push_str(&format!("+{}\n", new_lines[ni]));
                oi += 1;
                ni += 1;
            }
        } else if oi < old_lines.len() {
            output.push_str(&format!("-{}\n", old_lines[oi]));
            oi += 1;
        } else {
            output.push_str(&format!("+{}\n", new_lines[ni]));
            ni += 1;
        }
    }

    output
}

/// Run the diagnose subcommand
fn run_diagnose(args: &FixDiagnoseArgs, format: OutputFormat, lang: Option<&str>) -> Result<()> {
    let error_text = read_error_text(&args.error, &args.error_file, args.stdin)?;

    if let Some(surface_path) = &args.api_surface {
        eprintln!(
            "Note: API surface enrichment available from '{}'",
            surface_path.display()
        );
    }

    let source = std::fs::read_to_string(&args.source)
        .map_err(|e| anyhow!("Failed to read source file '{}': {}", args.source.display(), e))?;

    let diagnosis = fix::diagnose(&error_text, &source, lang, None);

    match diagnosis {
        Some(diag) => {
            let writer = OutputWriter::new(format, false);
            writer.write(&diag)?;
            Ok(())
        }
        None => Err(anyhow!(
            "Could not parse or diagnose the error. The error format may not be supported yet."
        )),
    }
}

/// Run the apply subcommand
fn run_apply(args: &FixApplyArgs, format: OutputFormat, lang: Option<&str>) -> Result<()> {
    let error_text = read_error_text(&args.error, &args.error_file, args.stdin)?;

    if let Some(surface_path) = &args.api_surface {
        eprintln!(
            "Note: API surface enrichment available from '{}'",
            surface_path.display()
        );
    }

    let source = std::fs::read_to_string(&args.source)
        .map_err(|e| anyhow!("Failed to read source file '{}': {}", args.source.display(), e))?;

    let diagnosis = fix::diagnose(&error_text, &source, lang, None)
        .ok_or_else(|| {
            anyhow!("Could not parse or diagnose the error. The error format may not be supported.")
        })?;

    match &diagnosis.fix {
        Some(fix_data) => {
            let patched = fix::apply_fix(&source, fix_data);

            if args.diff {
                // Show unified diff instead of full patched source
                match format {
                    OutputFormat::Json | OutputFormat::Compact => {
                        let diff_text = compute_line_diff(&source, &patched);
                        let result = serde_json::json!({
                            "diagnosis": diagnosis,
                            "diff": diff_text,
                        });
                        let writer = OutputWriter::new(format, false);
                        writer.write(&result)?;
                    }
                    _ => {
                        let diff_text = compute_line_diff(&source, &patched);
                        print!("{}", diff_text);
                    }
                }
                Ok(())
            } else if args.in_place {
                std::fs::write(&args.source, &patched).map_err(|e| {
                    anyhow!(
                        "Failed to write patched source to '{}': {}",
                        args.source.display(),
                        e
                    )
                })?;
                eprintln!("Fixed: {}", diagnosis.message);
                Ok(())
            } else if let Some(output_path) = &args.output {
                std::fs::write(output_path, &patched).map_err(|e| {
                    anyhow!(
                        "Failed to write patched source to '{}': {}",
                        output_path.display(),
                        e
                    )
                })?;
                eprintln!("Fixed: {}", diagnosis.message);
                Ok(())
            } else {
                // Write to stdout
                match format {
                    OutputFormat::Json | OutputFormat::Compact => {
                        let result = serde_json::json!({
                            "diagnosis": diagnosis,
                            "patched_source": patched,
                        });
                        let writer = OutputWriter::new(format, false);
                        writer.write(&result)?;
                    }
                    _ => {
                        // Text mode: just print the patched source
                        print!("{}", patched);
                    }
                }
                Ok(())
            }
        }
        None => {
            // No fix available -- print the diagnosis as advisory
            eprintln!(
                "No auto-fix available (confidence: {:?}). Diagnosis:",
                diagnosis.confidence
            );
            let writer = OutputWriter::new(format, false);
            writer.write(&diagnosis)?;
            // Exit with non-zero to indicate no fix was applied
            Err(anyhow!(
                "No deterministic fix available for this error. Escalate to a model."
            ))
        }
    }
}

/// Run the check subcommand: test -> diagnose -> fix -> repeat loop.
fn run_check(args: &FixCheckArgs, format: OutputFormat, lang: Option<&str>) -> Result<()> {
    use fix::{run_check_loop, CheckConfig};

    if !args.file.exists() {
        return Err(anyhow!(
            "Source file '{}' does not exist.",
            args.file.display()
        ));
    }

    let config = CheckConfig {
        file: &args.file,
        test_cmd: &args.test_cmd,
        lang,
        max_attempts: args.max_attempts,
    };

    let result = run_check_loop(&config);

    // Report results
    let writer = OutputWriter::new(format, false);
    writer.write(&result)?;

    if result.final_pass {
        eprintln!(
            "All errors fixed in {} iteration{}.",
            result.iterations,
            if result.iterations == 1 { "" } else { "s" }
        );
        Ok(())
    } else {
        Err(anyhow!(
            "Some errors could not be fixed after {} attempt{}.",
            result.iterations,
            if result.iterations == 1 { "" } else { "s" }
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_error_text_inline() {
        let text = read_error_text(
            &Some("NameError: name 'x' is not defined".to_string()),
            &None,
            false,
        )
        .unwrap();
        assert_eq!(text, "NameError: name 'x' is not defined");
    }

    #[test]
    fn test_read_error_text_file() {
        let dir = std::env::temp_dir().join("tldr_fix_test");
        std::fs::create_dir_all(&dir).unwrap();
        let err_file = dir.join("test_error.txt");
        std::fs::write(&err_file, "KeyError: 'name'").unwrap();

        let text = read_error_text(&None, &Some(err_file.clone()), false).unwrap();
        assert_eq!(text, "KeyError: 'name'");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_error_text_missing_file() {
        let result = read_error_text(
            &None,
            &Some(PathBuf::from("/nonexistent/path/error.txt")),
            false,
        );
        assert!(result.is_err());
    }

    // ---- Check subcommand tests ----

    #[test]
    fn test_fix_check_args_defaults() {
        // Verify the FixCheckArgs struct has correct field types
        let args = FixCheckArgs {
            file: PathBuf::from("app.py"),
            test_cmd: "pytest tests/".to_string(),
            max_attempts: 5,
        };
        assert_eq!(args.file, PathBuf::from("app.py"));
        assert_eq!(args.test_cmd, "pytest tests/");
        assert_eq!(args.max_attempts, 5);
    }

    #[test]
    fn test_fix_check_args_with_max_attempts() {
        let args = FixCheckArgs {
            file: PathBuf::from("main.rs"),
            test_cmd: "cargo test".to_string(),
            max_attempts: 10,
        };
        assert_eq!(args.max_attempts, 10);
    }

    #[test]
    fn test_fix_command_check_variant_exists() {
        // Ensure the Check variant exists on FixCommand
        let args = FixCheckArgs {
            file: PathBuf::from("app.py"),
            test_cmd: "pytest".to_string(),
            max_attempts: 5,
        };
        let cmd = FixCommand::Check(args);
        // Verify Debug representation contains "Check"
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("Check"), "FixCommand should have Check variant");
    }

    #[test]
    fn test_run_check_missing_file() {
        let args = FixCheckArgs {
            file: PathBuf::from("/nonexistent/file.py"),
            test_cmd: "true".to_string(),
            max_attempts: 5,
        };
        let result = run_check(&args, OutputFormat::Json, None);
        assert!(result.is_err(), "Should error on missing file");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("does not exist"),
            "Error should mention missing file: {}",
            err_msg
        );
    }

    #[test]
    fn test_run_check_succeeds_on_passing_test() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let source_path = dir.path().join("app.py");
        std::fs::write(&source_path, "x = 1\n").expect("write source");

        let args = FixCheckArgs {
            file: source_path,
            test_cmd: "true".to_string(),
            max_attempts: 5,
        };
        let result = run_check(&args, OutputFormat::Json, Some("python"));
        assert!(result.is_ok(), "Should succeed when test passes: {:?}", result);
    }

    // ---- Diff flag tests ----

    #[test]
    fn test_fix_apply_args_has_diff_field() {
        let args = FixApplyArgs {
            source: PathBuf::from("app.py"),
            error: Some("NameError: name 'x' is not defined".to_string()),
            error_file: None,
            output: None,
            stdin: false,
            in_place: false,
            diff: true,
            api_surface: None,
        };
        assert!(args.diff);
    }

    #[test]
    fn test_run_apply_diff_flag() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let source_path = dir.path().join("app.py");
        // Write a source file with a missing import that fix can handle
        std::fs::write(&source_path, "import os\nx = json.loads('{}')\n").expect("write source");

        let args = FixApplyArgs {
            source: source_path,
            error: Some("NameError: name 'json' is not defined".to_string()),
            error_file: None,
            output: None,
            stdin: false,
            in_place: false,
            diff: true,
            api_surface: None,
        };
        // Should succeed (produces diff output to stdout)
        let result = run_apply(&args, OutputFormat::Text, Some("python"));
        assert!(result.is_ok(), "run_apply with --diff should succeed: {:?}", result);
    }

    // ---- API surface flag tests ----

    #[test]
    fn test_fix_diagnose_args_has_api_surface_field() {
        let args = FixDiagnoseArgs {
            source: PathBuf::from("app.py"),
            error: Some("error".to_string()),
            error_file: None,
            stdin: false,
            api_surface: Some(PathBuf::from("surface.json")),
        };
        assert_eq!(args.api_surface, Some(PathBuf::from("surface.json")));
    }

    #[test]
    fn test_fix_apply_args_has_api_surface_field() {
        let args = FixApplyArgs {
            source: PathBuf::from("app.py"),
            error: Some("error".to_string()),
            error_file: None,
            output: None,
            stdin: false,
            in_place: false,
            diff: false,
            api_surface: Some(PathBuf::from("surface.json")),
        };
        assert_eq!(args.api_surface, Some(PathBuf::from("surface.json")));
    }

    #[test]
    fn test_run_check_fails_on_unfixable_error() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let source_path = dir.path().join("app.py");
        let script_path = dir.path().join("test.sh");

        std::fs::write(&source_path, "x = 1\n").expect("write source");
        std::fs::write(
            &script_path,
            "#!/bin/sh\necho 'just random junk' >&2\nexit 1\n",
        )
        .expect("write script");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod script");
        }

        let cmd = script_path.display().to_string();
        let args = FixCheckArgs {
            file: source_path,
            test_cmd: cmd,
            max_attempts: 3,
        };
        let result = run_check(&args, OutputFormat::Json, Some("python"));
        assert!(result.is_err(), "Should fail when error is unfixable");
    }
}
