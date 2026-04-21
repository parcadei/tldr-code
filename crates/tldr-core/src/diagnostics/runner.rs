//! Tool execution and detection module.
//!
//! This module handles:
//! - Detecting which diagnostic tools are available on PATH
//! - Running tools with timeout handling
//! - Parallel execution of multiple tools
//! - Capturing stdout/stderr and exit codes

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::diagnostics::parsers::*;
use crate::diagnostics::{
    Diagnostic, DiagnosticsReport, ToolConfig, ToolResult,
};
use crate::error::TldrError;
use crate::types::Language;

// =============================================================================
// Tool Detection
// =============================================================================

/// Check if a tool binary is available on PATH.
///
/// Uses `which` on Unix and `where` on Windows to check availability.
pub fn is_tool_available(binary: &str) -> bool {
    #[cfg(unix)]
    {
        Command::new("which")
            .arg(binary)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        Command::new("where")
            .arg(binary)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Get the version string of a tool, if available.
pub fn get_tool_version(binary: &str) -> Option<String> {
    let output = Command::new(binary).arg("--version").output().ok()?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout);
        // Extract first line and trim
        version.lines().next().map(|s| s.trim().to_string())
    } else {
        None
    }
}

/// Get all diagnostic tools configured for a language.
pub fn tools_for_language(lang: Language) -> Vec<ToolConfig> {
    match lang {
        Language::Python => vec![
            ToolConfig {
                name: "pyright",
                binary: "pyright",
                args: vec!["--outputjson".to_string()],
                is_type_checker: true,
                is_linter: false,
            },
            ToolConfig {
                name: "ruff",
                binary: "ruff",
                args: vec![
                    "check".to_string(),
                    "--output-format".to_string(),
                    "json".to_string(),
                ],
                is_type_checker: false,
                is_linter: true,
            },
        ],
        Language::TypeScript | Language::JavaScript => vec![
            ToolConfig {
                name: "tsc",
                binary: "tsc",
                args: vec![
                    "--noEmit".to_string(),
                    "--pretty".to_string(),
                    "false".to_string(),
                ],
                is_type_checker: true,
                is_linter: false,
            },
            ToolConfig {
                name: "eslint",
                binary: "eslint",
                args: vec!["-f".to_string(), "json".to_string()],
                is_type_checker: false,
                is_linter: true,
            },
        ],
        Language::Go => vec![
            ToolConfig {
                name: "go vet",
                binary: "go",
                args: vec!["vet".to_string(), "-json".to_string()],
                is_type_checker: true,
                is_linter: false,
            },
            ToolConfig {
                name: "golangci-lint",
                binary: "golangci-lint",
                args: vec![
                    "run".to_string(),
                    "--out-format".to_string(),
                    "json".to_string(),
                ],
                is_type_checker: false,
                is_linter: true,
            },
        ],
        Language::Rust => vec![
            ToolConfig {
                name: "cargo check",
                binary: "cargo",
                args: vec!["check".to_string(), "--message-format=json".to_string()],
                is_type_checker: true,
                is_linter: false,
            },
            ToolConfig {
                name: "clippy",
                binary: "cargo",
                args: vec!["clippy".to_string(), "--message-format=json".to_string()],
                is_type_checker: false,
                is_linter: true,
            },
        ],
        Language::Kotlin => vec![
            ToolConfig {
                name: "kotlinc",
                binary: "kotlinc",
                args: vec!["-language-version".to_string(), "1.9".to_string()],
                is_type_checker: true,
                is_linter: false,
            },
            ToolConfig {
                name: "detekt",
                binary: "detekt-cli",
                args: vec!["--report".to_string(), "txt:stdout".to_string()],
                is_type_checker: false,
                is_linter: true,
            },
        ],
        Language::Swift => vec![
            ToolConfig {
                name: "swiftc",
                binary: "swiftc",
                args: vec!["-typecheck".to_string()],
                is_type_checker: true,
                is_linter: false,
            },
            ToolConfig {
                name: "swiftlint",
                binary: "swiftlint",
                args: vec![
                    "lint".to_string(),
                    "--reporter".to_string(),
                    "json".to_string(),
                    "--quiet".to_string(),
                ],
                is_type_checker: false,
                is_linter: true,
            },
        ],
        Language::CSharp => vec![ToolConfig {
            name: "dotnet build",
            binary: "dotnet",
            args: vec![
                "build".to_string(),
                "--no-restore".to_string(),
                "--verbosity".to_string(),
                "quiet".to_string(),
            ],
            is_type_checker: true,
            is_linter: true, // Roslyn analyzers are built in
        }],
        Language::Scala => vec![ToolConfig {
            name: "scalac",
            binary: "scalac",
            args: vec![],
            is_type_checker: true,
            is_linter: false,
        }],
        Language::Elixir => vec![
            ToolConfig {
                name: "mix compile",
                binary: "mix",
                args: vec!["compile".to_string(), "--warnings-as-errors".to_string()],
                is_type_checker: true,
                is_linter: false,
            },
            ToolConfig {
                name: "credo",
                binary: "mix",
                args: vec![
                    "credo".to_string(),
                    "--format".to_string(),
                    "json".to_string(),
                ],
                is_type_checker: false,
                is_linter: true,
            },
        ],
        Language::Lua => vec![ToolConfig {
            name: "luacheck",
            binary: "luacheck",
            args: vec![
                "--formatter".to_string(),
                "plain".to_string(),
                "--no-color".to_string(),
            ],
            is_type_checker: false,
            is_linter: true,
        }],
        Language::Java => vec![
            ToolConfig {
                name: "javac",
                binary: "javac",
                args: vec!["-Xlint:all".to_string()],
                is_type_checker: true,
                is_linter: false,
            },
            ToolConfig {
                name: "checkstyle",
                binary: "checkstyle",
                args: vec!["-f".to_string(), "plain".to_string()],
                is_type_checker: false,
                is_linter: true,
            },
        ],
        Language::C | Language::Cpp => vec![
            ToolConfig {
                name: "clang",
                binary: "clang",
                args: vec!["-fsyntax-only".to_string(), "-Wall".to_string()],
                is_type_checker: true,
                is_linter: false,
            },
            ToolConfig {
                name: "clang-tidy",
                binary: "clang-tidy",
                args: vec![],
                is_type_checker: false,
                is_linter: true,
            },
        ],
        Language::Ruby => vec![ToolConfig {
            name: "rubocop",
            binary: "rubocop",
            args: vec!["--format".to_string(), "json".to_string()],
            is_type_checker: false,
            is_linter: true,
        }],
        Language::Php => vec![
            ToolConfig {
                name: "php",
                binary: "php",
                args: vec!["-l".to_string()],
                is_type_checker: true,
                is_linter: false,
            },
            ToolConfig {
                name: "phpstan",
                binary: "phpstan",
                args: vec![
                    "analyse".to_string(),
                    "--error-format=json".to_string(),
                    "--no-progress".to_string(),
                ],
                is_type_checker: false,
                is_linter: true,
            },
        ],
        _ => vec![],
    }
}

/// Detect which tools are available for a given language.
/// Only returns tools that are actually installed.
pub fn detect_available_tools(lang: Language) -> Vec<ToolConfig> {
    tools_for_language(lang)
        .into_iter()
        .filter(|t| is_tool_available(t.binary))
        .collect()
}

// =============================================================================
// Tool Execution
// =============================================================================

/// Run a single diagnostic tool and parse its output.
///
/// # Arguments
/// * `tool` - The tool configuration
/// * `path` - The path to analyze
/// * `timeout_secs` - Timeout in seconds
///
/// # Returns
/// A tuple of (ToolResult, Vec<Diagnostic>)
pub fn run_tool(
    tool: &ToolConfig,
    path: &Path,
    timeout_secs: u64,
) -> (ToolResult, Vec<Diagnostic>) {
    let start = Instant::now();

    // Build the command
    let mut cmd = Command::new(tool.binary);
    cmd.args(&tool.args);
    cmd.arg(path);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // Spawn the process
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return (
                ToolResult {
                    name: tool.name.to_string(),
                    version: None,
                    success: false,
                    duration_ms: start.elapsed().as_millis() as u64,
                    diagnostic_count: 0,
                    error: Some(format!("Failed to start {}: {}", tool.name, e)),
                },
                Vec::new(),
            );
        }
    };

    // Wait with timeout
    let timeout = Duration::from_secs(timeout_secs);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    break Err("Timeout");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => break Err(Box::leak(format!("{}", e).into_boxed_str()) as &str),
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    // Handle timeout or error
    let _exit_status = match status {
        Ok(s) => s,
        Err(e) => {
            return (
                ToolResult {
                    name: tool.name.to_string(),
                    version: get_tool_version(tool.binary),
                    success: false,
                    duration_ms,
                    diagnostic_count: 0,
                    error: Some(e.to_string()),
                },
                Vec::new(),
            );
        }
    };

    // Read stdout and stderr
    let mut stdout = String::new();
    let mut stderr = String::new();

    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }

    // Parse the output based on tool type
    let parse_result = parse_tool_output(tool.name, &stdout, &stderr);

    let (diagnostics, error) = match parse_result {
        Ok(diags) => (diags, None),
        Err(e) => {
            // Some tools exit non-zero when they find issues, which is OK
            // Only treat it as an error if we couldn't parse the output
            if !stdout.is_empty() || !stderr.is_empty() {
                // Try to parse anyway for tools that might output to stderr
                let fallback = parse_tool_output(tool.name, &stderr, &stdout);
                match fallback {
                    Ok(diags) => (diags, None),
                    Err(_) => (Vec::new(), Some(format!("Parse error: {}", e))),
                }
            } else {
                (Vec::new(), Some(format!("Parse error: {}", e)))
            }
        }
    };

    let diagnostic_count = diagnostics.len();

    // Tool is successful if it ran and we could parse output (even if it found issues)
    let success = error.is_none();

    (
        ToolResult {
            name: tool.name.to_string(),
            version: get_tool_version(tool.binary),
            success,
            duration_ms,
            diagnostic_count,
            error,
        },
        diagnostics,
    )
}

/// Parse output based on tool name.
fn parse_tool_output(
    tool_name: &str,
    stdout: &str,
    _stderr: &str,
) -> Result<Vec<Diagnostic>, TldrError> {
    match tool_name {
        "pyright" => parse_pyright_output(stdout),
        "ruff" => parse_ruff_output(stdout),
        "tsc" => parse_tsc_text(stdout),
        "eslint" => parse_eslint_output(stdout),
        "cargo check" | "clippy" => parse_cargo_output(stdout),
        "go vet" => parse_go_vet_output(stdout),
        "golangci-lint" => parse_golangci_lint_output(stdout),
        "kotlinc" => parse_kotlinc_output(stdout),
        "detekt" => parse_detekt_output(stdout),
        "swiftc" => parse_swiftc_output(stdout),
        "swiftlint" => parse_swiftlint_output(stdout),
        "dotnet build" => parse_dotnet_build_output(stdout),
        "scalac" => parse_scalac_output(stdout),
        "mix compile" => parse_mix_compile_output(stdout),
        "credo" => parse_credo_output(stdout),
        "luacheck" => parse_luacheck_output(stdout),
        "javac" => parse_javac_output(stdout),
        "checkstyle" => parse_checkstyle_output(stdout),
        "clang" => parse_clang_output(stdout, "clang"),
        "clang-tidy" => parse_clang_output(stdout, "clang-tidy"),
        "rubocop" => parse_rubocop_output(stdout),
        "php" => parse_php_lint_output(stdout),
        "phpstan" => parse_phpstan_output(stdout),
        _ => Err(TldrError::ParseError {
            file: std::path::PathBuf::from(format!("<{}-output>", tool_name)),
            line: None,
            message: format!("Unknown tool: {}", tool_name),
        }),
    }
}

/// Run multiple tools in parallel (or sequentially on single-core systems).
///
/// # Arguments
/// * `tools` - The tools to run
/// * `path` - The path to analyze
/// * `timeout_secs` - Timeout per tool in seconds
///
/// # Returns
/// A DiagnosticsReport with results from all tools.
pub fn run_tools_parallel(
    tools: &[ToolConfig],
    path: &Path,
    timeout_secs: u64,
) -> Result<DiagnosticsReport, TldrError> {
    use std::sync::mpsc;
    use std::thread;

    if tools.is_empty() {
        return Err(TldrError::ParseError {
            file: std::path::PathBuf::from("<diagnostics>"),
            line: None,
            message: "No tools provided".to_string(),
        });
    }

    // Check core count - run sequentially if single core
    let num_cpus = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let mut all_diagnostics = Vec::new();
    let mut all_results = Vec::new();

    if num_cpus <= 1 || tools.len() == 1 {
        // Sequential execution
        for tool in tools {
            let (result, diags) = run_tool(tool, path, timeout_secs);
            all_results.push(result);
            all_diagnostics.extend(diags);
        }
    } else {
        // Parallel execution
        let (tx, rx) = mpsc::channel();
        let path = path.to_path_buf();

        let handles: Vec<_> = tools
            .iter()
            .map(|tool| {
                let tx = tx.clone();
                let tool = tool.clone();
                let path = path.clone();

                thread::spawn(move || {
                    let (result, diags) = run_tool(&tool, &path, timeout_secs);
                    let _ = tx.send((result, diags));
                })
            })
            .collect();

        // Drop the original sender so rx.iter() terminates
        drop(tx);

        // Collect results
        for (result, diags) in rx.iter() {
            all_results.push(result);
            all_diagnostics.extend(diags);
        }

        // Wait for all threads
        for handle in handles {
            let _ = handle.join();
        }
    }

    // Compute summary
    let summary = crate::diagnostics::compute_summary(&all_diagnostics);

    Ok(DiagnosticsReport {
        diagnostics: all_diagnostics,
        summary,
        tools_run: all_results,
        files_analyzed: 1, // This would need proper counting
    })
}

/// Get install suggestions for missing tools.
pub fn get_install_suggestion(tool_name: &str) -> &'static str {
    match tool_name {
        "pyright" => "pip install pyright",
        "ruff" => "pip install ruff",
        "tsc" => "npm install -g typescript",
        "eslint" => "npm install -g eslint",
        "golangci-lint" => "go install github.com/golangci/golangci-lint/cmd/golangci-lint@latest",
        "cargo" | "clippy" => "rustup component add clippy",
        "kotlinc" => "Install Kotlin: https://kotlinlang.org/docs/command-line.html",
        "detekt" | "detekt-cli" => "Install detekt: https://detekt.dev/docs/gettingstarted/cli",
        "swiftc" => "Install Xcode or Swift toolchain: https://swift.org/download/",
        "swiftlint" => "brew install swiftlint",
        "dotnet" => "Install .NET SDK: https://dotnet.microsoft.com/download",
        "scalac" => "Install Scala: https://www.scala-lang.org/download/",
        "mix" => "Install Elixir: https://elixir-lang.org/install.html",
        "luacheck" => "luarocks install luacheck",
        "javac" => "Install JDK: https://adoptium.net/",
        "checkstyle" => "Install Checkstyle: https://checkstyle.org/",
        "clang" => "Install LLVM/Clang: https://releases.llvm.org/ or brew install llvm",
        "clang-tidy" => "Install LLVM/Clang: https://releases.llvm.org/ or brew install llvm",
        "rubocop" => "gem install rubocop",
        "php" => "Install PHP: https://www.php.net/downloads",
        "phpstan" => "composer require --dev phpstan/phpstan",
        _ => "Check tool documentation",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_tool_available_which() {
        // 'which' should be available on Unix systems
        #[cfg(unix)]
        assert!(is_tool_available("which"));
    }

    #[test]
    fn test_is_tool_unavailable() {
        assert!(!is_tool_available("nonexistent_tool_xyz_12345"));
    }

    #[test]
    fn test_tools_for_python() {
        let tools = tools_for_language(Language::Python);
        assert!(tools.iter().any(|t| t.name == "pyright"));
        assert!(tools.iter().any(|t| t.name == "ruff"));
    }

    #[test]
    fn test_tools_for_typescript() {
        let tools = tools_for_language(Language::TypeScript);
        assert!(tools.iter().any(|t| t.name == "tsc"));
        assert!(tools.iter().any(|t| t.name == "eslint"));
    }

    #[test]
    fn test_tools_for_rust() {
        let tools = tools_for_language(Language::Rust);
        assert!(tools.iter().any(|t| t.name == "cargo check"));
        assert!(tools.iter().any(|t| t.name == "clippy"));
    }

    #[test]
    fn test_tools_for_go() {
        let tools = tools_for_language(Language::Go);
        assert!(tools.iter().any(|t| t.name == "go vet"));
        assert!(tools.iter().any(|t| t.name == "golangci-lint"));
    }

    #[test]
    fn test_tools_for_kotlin() {
        let tools = tools_for_language(Language::Kotlin);
        assert!(tools.iter().any(|t| t.name == "kotlinc"));
        assert!(tools.iter().any(|t| t.name == "detekt"));
    }

    #[test]
    fn test_tools_for_swift() {
        let tools = tools_for_language(Language::Swift);
        assert!(tools.iter().any(|t| t.name == "swiftc"));
        assert!(tools.iter().any(|t| t.name == "swiftlint"));
    }

    #[test]
    fn test_tools_for_csharp() {
        let tools = tools_for_language(Language::CSharp);
        assert!(tools.iter().any(|t| t.name == "dotnet build"));
    }

    #[test]
    fn test_tools_for_scala() {
        let tools = tools_for_language(Language::Scala);
        assert!(tools.iter().any(|t| t.name == "scalac"));
    }

    #[test]
    fn test_tools_for_elixir() {
        let tools = tools_for_language(Language::Elixir);
        assert!(tools.iter().any(|t| t.name == "mix compile"));
        assert!(tools.iter().any(|t| t.name == "credo"));
    }

    #[test]
    fn test_tools_for_lua() {
        let tools = tools_for_language(Language::Lua);
        assert!(tools.iter().any(|t| t.name == "luacheck"));
    }

    #[test]
    fn test_install_suggestions() {
        assert!(get_install_suggestion("pyright").contains("pip"));
        assert!(get_install_suggestion("eslint").contains("npm"));
    }

    #[test]
    fn test_install_suggestions_new_languages() {
        assert!(get_install_suggestion("kotlinc").contains("kotlin"));
        assert!(get_install_suggestion("swiftlint").contains("brew"));
        assert!(get_install_suggestion("dotnet").contains(".NET"));
        assert!(get_install_suggestion("scalac").contains("scala"));
        assert!(
            get_install_suggestion("mix").contains("elixir")
                || get_install_suggestion("mix").contains("Elixir")
        );
        assert!(get_install_suggestion("luacheck").contains("luarocks"));
    }

    #[test]
    fn test_tools_for_java() {
        let tools = tools_for_language(Language::Java);
        assert!(tools.iter().any(|t| t.name == "javac"));
        assert!(tools.iter().any(|t| t.name == "checkstyle"));
    }

    #[test]
    fn test_tools_for_c() {
        let tools = tools_for_language(Language::C);
        assert!(tools.iter().any(|t| t.name == "clang"));
        assert!(tools.iter().any(|t| t.name == "clang-tidy"));
    }

    #[test]
    fn test_tools_for_cpp() {
        let tools = tools_for_language(Language::Cpp);
        assert!(tools.iter().any(|t| t.name == "clang"));
        assert!(tools.iter().any(|t| t.name == "clang-tidy"));
    }

    #[test]
    fn test_tools_for_ruby() {
        let tools = tools_for_language(Language::Ruby);
        assert!(tools.iter().any(|t| t.name == "rubocop"));
    }

    #[test]
    fn test_tools_for_php() {
        let tools = tools_for_language(Language::Php);
        assert!(tools.iter().any(|t| t.name == "php"));
        assert!(tools.iter().any(|t| t.name == "phpstan"));
    }

    #[test]
    fn test_install_suggestions_java_c_ruby_php() {
        assert!(get_install_suggestion("javac").contains("JDK"));
        assert!(get_install_suggestion("checkstyle").contains("Checkstyle"));
        assert!(
            get_install_suggestion("clang").contains("LLVM")
                || get_install_suggestion("clang").contains("llvm")
        );
        assert!(
            get_install_suggestion("clang-tidy").contains("LLVM")
                || get_install_suggestion("clang-tidy").contains("llvm")
        );
        assert!(get_install_suggestion("rubocop").contains("gem"));
        assert!(
            get_install_suggestion("php").contains("PHP")
                || get_install_suggestion("php").contains("php")
        );
        assert!(get_install_suggestion("phpstan").contains("composer"));
    }
}
