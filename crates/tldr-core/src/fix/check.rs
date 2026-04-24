//! Fix check loop -- run command, diagnose, fix, repeat.
//!
//! This module implements the `tldr fix check` core logic: a loop that
//! runs a test command, parses any errors from its output, diagnoses them,
//! applies fixes, and repeats until the test passes or the maximum number
//! of attempts is reached.
//!
//! # Usage
//!
//! ```rust,ignore
//! use tldr_core::fix::check::{run_check_loop, CheckConfig};
//! use std::path::Path;
//!
//! let config = CheckConfig {
//!     file: Path::new("src/app.py"),
//!     test_cmd: "pytest tests/test_app.py",
//!     lang: None,
//!     max_attempts: 5,
//! };
//! let result = run_check_loop(&config);
//! if result.final_pass {
//!     println!("All errors fixed in {} iterations!", result.iterations);
//! }
//! ```

use std::path::Path;
use std::process::Command;

use super::diagnose;
use super::error_parser::parse_error;
use super::patch::apply_fix;
use super::types::Diagnosis;

/// Result of a single fix attempt in the check loop.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FixAttempt {
    /// Which iteration of the loop this attempt was (1-indexed).
    pub iteration: usize,
    /// The error code that was diagnosed (e.g., "NameError", "E0599").
    pub error_code: String,
    /// The error message that was diagnosed.
    pub message: String,
    /// Whether this attempt successfully produced and applied a fix.
    pub fixed: bool,
    /// Description of the fix that was applied, if any.
    pub description: Option<String>,
}

/// Result of the entire check loop.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CheckResult {
    /// The file that was being fixed.
    pub file: String,
    /// The test command that was run.
    pub test_cmd: String,
    /// Details of each fix attempt.
    pub attempts: Vec<FixAttempt>,
    /// Whether the test command passed after all attempts.
    pub final_pass: bool,
    /// Total number of iterations performed.
    pub iterations: usize,
}

/// Configuration for the check loop.
pub struct CheckConfig<'a> {
    /// Path to the source file to fix.
    pub file: &'a Path,
    /// Shell command to run as the test (e.g., "pytest tests/test_app.py").
    pub test_cmd: &'a str,
    /// Optional language hint; auto-detected from file extension if `None`.
    pub lang: Option<&'a str>,
    /// Maximum number of fix attempts before giving up (default: 5).
    pub max_attempts: usize,
}

/// Detect language from a file extension.
///
/// Returns a language string suitable for the `fix::diagnose` system,
/// or `None` if the extension is not recognized.
fn detect_lang_from_extension(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "py" => Some("python"),
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "go" => Some("go"),
        "js" | "mjs" => Some("javascript"),
        _ => None,
    }
}

/// Run a shell command and capture its output.
///
/// Returns `(exit_success, combined_error_output)` where the error output
/// is stderr if non-empty, otherwise stdout (some tools write errors to stdout).
fn run_command(cmd: &str) -> (bool, String) {
    let output = Command::new("sh").arg("-c").arg(cmd).output();

    match output {
        Ok(out) => {
            let success = out.status.success();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            // Prefer stderr for error output; fall back to stdout
            let error_output = if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            };
            (success, error_output)
        }
        Err(e) => (false, format!("Failed to execute command: {}", e)),
    }
}

/// Run the fix check loop.
///
/// Executes `config.test_cmd`, and if it fails:
/// 1. Parses the error output
/// 2. Reads the source file
/// 3. Diagnoses the error
/// 4. Applies the fix (if available) and writes it back
/// 5. Repeats until the test passes or `max_attempts` is reached
///
/// Returns a `CheckResult` with details of each attempt.
pub fn run_check_loop(config: &CheckConfig) -> CheckResult {
    let lang = config
        .lang
        .or_else(|| detect_lang_from_extension(config.file));

    let file_str = config.file.display().to_string();
    let mut attempts: Vec<FixAttempt> = Vec::new();
    let mut iteration = 0;
    let mut final_pass = false;

    loop {
        iteration += 1;

        // Step 1: Run the test command
        let (success, error_output) = run_command(config.test_cmd);

        if success {
            final_pass = true;
            break;
        }

        // Stop if we have exhausted attempts
        if iteration > config.max_attempts {
            break;
        }

        // Step 2: Parse the error output
        let parsed = match parse_error(&error_output, lang) {
            Some(p) => p,
            None => {
                // Could not parse the error -- nothing we can fix
                attempts.push(FixAttempt {
                    iteration,
                    error_code: "unparseable".to_string(),
                    message: truncate_output(&error_output, 200),
                    fixed: false,
                    description: None,
                });
                break;
            }
        };

        // Step 3: Read the source file
        let source = match std::fs::read_to_string(config.file) {
            Ok(s) => s,
            Err(e) => {
                attempts.push(FixAttempt {
                    iteration,
                    error_code: parsed.error_type.clone(),
                    message: format!("Cannot read source file: {}", e),
                    fixed: false,
                    description: None,
                });
                break;
            }
        };

        // Step 4: Diagnose
        let diagnosis: Option<Diagnosis> = diagnose(&error_output, &source, lang, None);

        match diagnosis {
            Some(diag) if diag.fix.is_some() => {
                let fix = diag.fix.as_ref().unwrap();
                let patched = apply_fix(&source, fix);

                // Step 5: Write the patched source back
                match std::fs::write(config.file, &patched) {
                    Ok(()) => {
                        attempts.push(FixAttempt {
                            iteration,
                            error_code: diag.error_code.clone(),
                            message: diag.message.clone(),
                            fixed: true,
                            description: Some(fix.description.clone()),
                        });
                    }
                    Err(e) => {
                        attempts.push(FixAttempt {
                            iteration,
                            error_code: diag.error_code.clone(),
                            message: format!("Failed to write patched source: {}", e),
                            fixed: false,
                            description: None,
                        });
                        break;
                    }
                }
            }
            Some(diag) => {
                // Diagnosis exists but no fix available
                attempts.push(FixAttempt {
                    iteration,
                    error_code: diag.error_code.clone(),
                    message: diag.message.clone(),
                    fixed: false,
                    description: None,
                });
                break;
            }
            None => {
                // Could not diagnose
                attempts.push(FixAttempt {
                    iteration,
                    error_code: parsed.error_type.clone(),
                    message: format!("Could not diagnose: {}", parsed.message),
                    fixed: false,
                    description: None,
                });
                break;
            }
        }
    }

    CheckResult {
        file: file_str,
        test_cmd: config.test_cmd.to_string(),
        attempts,
        final_pass,
        iterations: iteration,
    }
}

/// Truncate output to a maximum length for display in attempt messages.
fn truncate_output(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.trim().to_string()
    } else {
        format!("{}...", &s[..max_len].trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Helper: create a temp directory with a source file and a test script.
    /// Returns (temp_dir, source_path, test_script_path).
    fn setup_temp_env(
        source_name: &str,
        source_content: &str,
        script_content: &str,
    ) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let source_path = dir.path().join(source_name);
        let script_path = dir.path().join("test.sh");

        std::fs::write(&source_path, source_content).expect("write source");

        let mut script = std::fs::File::create(&script_path).expect("create script");
        script
            .write_all(script_content.as_bytes())
            .expect("write script");

        // Make script executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod script");
        }

        (dir, source_path, script_path)
    }

    // ---- Language detection tests ----

    #[test]
    fn test_detect_lang_python() {
        assert_eq!(
            detect_lang_from_extension(Path::new("app.py")),
            Some("python")
        );
    }

    #[test]
    fn test_detect_lang_rust() {
        assert_eq!(
            detect_lang_from_extension(Path::new("main.rs")),
            Some("rust")
        );
    }

    #[test]
    fn test_detect_lang_typescript() {
        assert_eq!(
            detect_lang_from_extension(Path::new("app.ts")),
            Some("typescript")
        );
        assert_eq!(
            detect_lang_from_extension(Path::new("App.tsx")),
            Some("typescript")
        );
    }

    #[test]
    fn test_detect_lang_go() {
        assert_eq!(detect_lang_from_extension(Path::new("main.go")), Some("go"));
    }

    #[test]
    fn test_detect_lang_javascript() {
        assert_eq!(
            detect_lang_from_extension(Path::new("app.js")),
            Some("javascript")
        );
        assert_eq!(
            detect_lang_from_extension(Path::new("module.mjs")),
            Some("javascript")
        );
    }

    #[test]
    fn test_detect_lang_unknown() {
        assert_eq!(detect_lang_from_extension(Path::new("file.rb")), None);
        assert_eq!(detect_lang_from_extension(Path::new("no_ext")), None);
    }

    // ---- Loop termination tests ----

    #[test]
    fn test_check_loop_terminates_on_success() {
        // Test command that succeeds immediately
        let (_dir, source_path, _script_path) = setup_temp_env("app.py", "x = 1\n", "");

        let config = CheckConfig {
            file: &source_path,
            test_cmd: "true", // always succeeds
            lang: Some("python"),
            max_attempts: 5,
        };

        let result = run_check_loop(&config);
        assert!(result.final_pass, "Should pass immediately");
        assert_eq!(result.iterations, 1);
        assert!(result.attempts.is_empty(), "No fix attempts needed");
    }

    #[test]
    fn test_check_loop_terminates_on_no_fix_available() {
        // Test command that fails with an error we can parse but not fix
        let (_dir, source_path, script_path) = setup_temp_env(
            "app.py",
            "x = 1\n",
            "#!/bin/sh\necho 'RecursionError: maximum recursion depth exceeded' >&2\nexit 1\n",
        );

        let cmd = script_path.display().to_string();
        let config = CheckConfig {
            file: &source_path,
            test_cmd: &cmd,
            lang: Some("python"),
            max_attempts: 5,
        };

        let result = run_check_loop(&config);
        assert!(!result.final_pass, "Should not pass -- no fix available");
        assert_eq!(result.attempts.len(), 1, "Should try once then stop");
        assert!(!result.attempts[0].fixed, "Attempt should not be fixed");
    }

    #[test]
    fn test_check_loop_terminates_on_unparseable_error() {
        // Test command that fails with output we cannot parse
        let (_dir, source_path, script_path) = setup_temp_env(
            "app.py",
            "x = 1\n",
            "#!/bin/sh\necho 'just some random junk' >&2\nexit 1\n",
        );

        let cmd = script_path.display().to_string();
        let config = CheckConfig {
            file: &source_path,
            test_cmd: &cmd,
            lang: Some("python"),
            max_attempts: 5,
        };

        let result = run_check_loop(&config);
        assert!(!result.final_pass, "Should not pass -- unparseable error");
        assert_eq!(result.attempts.len(), 1);
        assert_eq!(result.attempts[0].error_code, "unparseable");
    }

    #[test]
    fn test_check_loop_respects_max_attempts() {
        // Script that always fails with a fixable error, but the fix
        // doesn't actually resolve the issue (so the loop runs until max_attempts).
        // We use a NameError that diagnoses to "add import json",
        // but the test script always fails regardless.
        let (_dir, source_path, script_path) = setup_temp_env(
            "app.py",
            "def f():\n    data = json.loads('{}')\n",
            "#!/bin/sh\necho \"NameError: name 'json' is not defined\" >&2\nexit 1\n",
        );

        let cmd = script_path.display().to_string();
        let config = CheckConfig {
            file: &source_path,
            test_cmd: &cmd,
            lang: Some("python"),
            max_attempts: 3,
        };

        let result = run_check_loop(&config);
        assert!(!result.final_pass, "Should not pass -- always fails");
        // First iteration applies a fix (import json), subsequent iterations
        // may or may not parse (the error stays the same but source changed).
        // The important thing is we don't exceed max_attempts.
        assert!(
            result.iterations <= 4, // max_attempts + 1 for final check
            "Should not exceed max_attempts + 1: got {}",
            result.iterations
        );
    }

    #[test]
    fn test_check_loop_applies_fix_and_passes() {
        // Create a script that fails on first run then succeeds.
        // We use a marker file to track state.
        let dir = tempfile::tempdir().expect("create temp dir");
        let source_path = dir.path().join("app.py");
        let marker_path = dir.path().join("marker");
        let script_path = dir.path().join("test.sh");

        // Source with a NameError-triggering pattern
        std::fs::write(&source_path, "def f():\n    data = json.loads('{}')\n")
            .expect("write source");

        // Script: fail on first run (no marker), succeed on subsequent runs
        let script = format!(
            "#!/bin/sh\nif [ -f \"{}\" ]; then\n  exit 0\nelse\n  touch \"{}\"\n  echo \"NameError: name 'json' is not defined\" >&2\n  exit 1\nfi\n",
            marker_path.display(),
            marker_path.display()
        );
        let mut f = std::fs::File::create(&script_path).expect("create script");
        f.write_all(script.as_bytes()).expect("write script");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod script");
        }

        let cmd = script_path.display().to_string();
        let config = CheckConfig {
            file: &source_path,
            test_cmd: &cmd,
            lang: Some("python"),
            max_attempts: 5,
        };

        let result = run_check_loop(&config);
        assert!(result.final_pass, "Should pass after fix");
        assert_eq!(result.iterations, 2, "Should take 2 iterations");
        assert_eq!(result.attempts.len(), 1, "One fix attempt");
        assert!(result.attempts[0].fixed, "Fix should have been applied");

        // Verify the source file was actually modified
        let patched = std::fs::read_to_string(&source_path).expect("read patched");
        assert!(
            patched.contains("import json"),
            "Source should contain the fix: got {:?}",
            patched
        );
    }

    #[test]
    fn test_check_result_serialization() {
        let result = CheckResult {
            file: "app.py".to_string(),
            test_cmd: "pytest".to_string(),
            attempts: vec![FixAttempt {
                iteration: 1,
                error_code: "NameError".to_string(),
                message: "name 'json' is not defined".to_string(),
                fixed: true,
                description: Some("Add import json".to_string()),
            }],
            final_pass: true,
            iterations: 2,
        };

        let json = serde_json::to_string(&result).expect("serialize");
        assert!(json.contains("NameError"));
        assert!(json.contains("final_pass"));
        assert!(json.contains("\"iterations\":2"));
    }

    #[test]
    fn test_fix_attempt_serialization() {
        let attempt = FixAttempt {
            iteration: 1,
            error_code: "E0599".to_string(),
            message: "no method found".to_string(),
            fixed: false,
            description: None,
        };

        let json = serde_json::to_string(&attempt).expect("serialize");
        assert!(json.contains("E0599"));
        assert!(json.contains("\"fixed\":false"));
    }

    #[test]
    fn test_truncate_output_short() {
        assert_eq!(truncate_output("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_output_long() {
        let long = "a".repeat(300);
        let result = truncate_output(&long, 200);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 204); // 200 chars + "..."
    }

    #[test]
    fn test_run_command_success() {
        let (success, _output) = run_command("true");
        assert!(success);
    }

    #[test]
    fn test_run_command_failure() {
        let (success, _output) = run_command("false");
        assert!(!success);
    }

    #[test]
    fn test_run_command_captures_stderr() {
        let (success, output) = run_command("echo 'error text' >&2; exit 1");
        assert!(!success);
        assert!(
            output.contains("error text"),
            "Should capture stderr: got {:?}",
            output
        );
    }

    #[test]
    fn test_run_command_falls_back_to_stdout() {
        let (_, output) = run_command("echo 'stdout error'; exit 1");
        assert!(
            output.contains("stdout error"),
            "Should fall back to stdout when stderr is empty: got {:?}",
            output
        );
    }

    #[test]
    fn test_check_loop_file_display_in_result() {
        let (_dir, source_path, _) = setup_temp_env("app.py", "x = 1\n", "");

        let config = CheckConfig {
            file: &source_path,
            test_cmd: "true",
            lang: Some("python"),
            max_attempts: 5,
        };

        let result = run_check_loop(&config);
        assert!(
            result.file.contains("app.py"),
            "Result should contain file path"
        );
        assert_eq!(result.test_cmd, "true");
    }
}
