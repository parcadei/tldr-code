//! Doctor command - Check and install diagnostic tools
//!
//! Provides tool detection and installation for supported languages:
//! - Detects type checkers and linters for 16 languages
//! - Reports installation status with paths
//! - Can auto-install tools for some languages
//!
//! # Example
//!
//! ```bash
//! # Check all diagnostic tools
//! tldr doctor
//!
//! # Check with JSON output
//! tldr doctor -f json
//!
//! # Install tools for a language
//! tldr doctor --install python
//! ```

use std::collections::BTreeMap;
use std::process::Command;

use anyhow::{bail, Result};
use clap::Args;
use serde::Serialize;

use crate::output::{OutputFormat, OutputWriter};

/// Tool information: (tool_name, install_instructions)
type ToolDef = (&'static str, &'static str);

/// Language tool definitions: type_checker and linter for each language
struct LangTools {
    type_checker: Option<ToolDef>,
    linter: Option<ToolDef>,
}

/// Tool definitions for all supported languages
fn get_tool_info() -> BTreeMap<&'static str, LangTools> {
    let mut tools = BTreeMap::new();

    tools.insert(
        "python",
        LangTools {
            type_checker: Some(("pyright", "pip install pyright  OR  npm install -g pyright")),
            linter: Some(("ruff", "pip install ruff")),
        },
    );

    tools.insert(
        "typescript",
        LangTools {
            type_checker: Some(("tsc", "npm install -g typescript")),
            linter: None,
        },
    );

    tools.insert(
        "javascript",
        LangTools {
            type_checker: None,
            linter: Some(("eslint", "npm install -g eslint")),
        },
    );

    tools.insert("go", LangTools {
        type_checker: Some(("go", "https://go.dev/dl/")),
        linter: Some(("golangci-lint", "brew install golangci-lint  OR  go install github.com/golangci/golangci-lint/cmd/golangci-lint@latest")),
    });

    tools.insert(
        "rust",
        LangTools {
            type_checker: Some(("cargo", "https://rustup.rs/")),
            linter: Some(("cargo-clippy", "rustup component add clippy")),
        },
    );

    tools.insert(
        "java",
        LangTools {
            type_checker: Some(("javac", "Install JDK: https://adoptium.net/")),
            linter: Some((
                "checkstyle",
                "brew install checkstyle  OR  download from checkstyle.org",
            )),
        },
    );

    tools.insert(
        "c",
        LangTools {
            type_checker: Some(("gcc", "xcode-select --install  OR  apt install gcc")),
            linter: Some((
                "cppcheck",
                "brew install cppcheck  OR  apt install cppcheck",
            )),
        },
    );

    tools.insert(
        "cpp",
        LangTools {
            type_checker: Some(("g++", "xcode-select --install  OR  apt install g++")),
            linter: Some((
                "cppcheck",
                "brew install cppcheck  OR  apt install cppcheck",
            )),
        },
    );

    tools.insert(
        "ruby",
        LangTools {
            type_checker: None,
            linter: Some(("rubocop", "gem install rubocop")),
        },
    );

    tools.insert(
        "php",
        LangTools {
            type_checker: None,
            linter: Some(("phpstan", "composer global require phpstan/phpstan")),
        },
    );

    tools.insert(
        "kotlin",
        LangTools {
            type_checker: Some(("kotlinc", "brew install kotlin  OR  sdk install kotlin")),
            linter: Some(("ktlint", "brew install ktlint")),
        },
    );

    tools.insert(
        "swift",
        LangTools {
            type_checker: Some(("swiftc", "xcode-select --install")),
            linter: Some(("swiftlint", "brew install swiftlint")),
        },
    );

    tools.insert(
        "csharp",
        LangTools {
            type_checker: Some(("dotnet", "https://dotnet.microsoft.com/download")),
            linter: None,
        },
    );

    tools.insert(
        "scala",
        LangTools {
            type_checker: Some(("scalac", "brew install scala  OR  sdk install scala")),
            linter: None,
        },
    );

    tools.insert(
        "elixir",
        LangTools {
            type_checker: Some(("elixir", "brew install elixir  OR  asdf install elixir")),
            linter: Some(("mix", "Included with Elixir")),
        },
    );

    tools.insert(
        "lua",
        LangTools {
            type_checker: None,
            linter: Some(("luacheck", "luarocks install luacheck")),
        },
    );

    tools
}

/// Install commands for languages that support auto-install
fn get_install_commands() -> BTreeMap<&'static str, Vec<&'static str>> {
    let mut cmds = BTreeMap::new();

    cmds.insert("python", vec!["pip", "install", "pyright", "ruff"]);
    cmds.insert(
        "go",
        vec![
            "go",
            "install",
            "github.com/golangci/golangci-lint/cmd/golangci-lint@latest",
        ],
    );
    cmds.insert("rust", vec!["rustup", "component", "add", "clippy"]);
    cmds.insert("ruby", vec!["gem", "install", "rubocop"]);
    cmds.insert("kotlin", vec!["brew", "install", "kotlin", "ktlint"]);
    cmds.insert("swift", vec!["brew", "install", "swiftlint"]);
    cmds.insert("lua", vec!["luarocks", "install", "luacheck"]);

    cmds
}

/// Tool status in JSON output
#[derive(Debug, Serialize)]
struct ToolStatus {
    name: String,
    installed: bool,
    path: Option<String>,
    install: Option<String>,
}

/// Language status in JSON output
#[derive(Debug, Serialize)]
struct LangStatus {
    type_checker: Option<ToolStatus>,
    linter: Option<ToolStatus>,
}

/// Check and install diagnostic tools for supported languages
///
/// Unlike most tldr commands, doctor defaults to text output for better UX.
/// Use `-f json` to get JSON output.
#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Install diagnostic tools for a specific language
    #[arg(long)]
    pub install: Option<String>,
}

impl DoctorArgs {
    /// Run the doctor command
    ///
    /// Note: Doctor defaults to text format for better UX (diagnostic output is meant to be
    /// human-readable). Use `-f json -q` to get JSON output.
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        if let Some(lang) = &self.install {
            self.run_install(lang)
        } else {
            self.run_check(format, quiet)
        }
    }

    /// Run install mode - install tools for a specific language
    fn run_install(&self, lang: &str) -> Result<()> {
        let lang_lower = lang.to_lowercase();
        let install_commands = get_install_commands();

        let Some(cmd_args) = install_commands.get(lang_lower.as_str()) else {
            let available: Vec<&str> = install_commands.keys().copied().collect();
            bail!(
                "No auto-install available for '{}'. Available: {}. unknown language.",
                lang,
                available.join(", ")
            );
        };

        eprintln!(
            "Installing tools for {}: {}",
            lang_lower,
            cmd_args.join(" ")
        );

        let status = Command::new(cmd_args[0]).args(&cmd_args[1..]).status();

        match status {
            Ok(exit_status) if exit_status.success() => {
                eprintln!("Installed {} tools", lang_lower);
                Ok(())
            }
            Ok(exit_status) => {
                bail!("Install failed with exit code: {:?}", exit_status.code());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                bail!("Command not found: {}", cmd_args[0]);
            }
            Err(e) => {
                bail!("Install failed: {}", e);
            }
        }
    }

    /// Run check mode - detect all diagnostic tools
    fn run_check(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);
        let tool_info = get_tool_info();

        let mut results: BTreeMap<String, LangStatus> = BTreeMap::new();

        for (lang, tools) in &tool_info {
            let type_checker = tools.type_checker.map(|(name, install_cmd)| {
                let path = which::which(name).ok().map(|p| p.display().to_string());
                let installed = path.is_some();
                ToolStatus {
                    name: name.to_string(),
                    installed,
                    path,
                    install: if installed {
                        None
                    } else {
                        Some(install_cmd.to_string())
                    },
                }
            });

            let linter = tools.linter.map(|(name, install_cmd)| {
                let path = which::which(name).ok().map(|p| p.display().to_string());
                let installed = path.is_some();
                ToolStatus {
                    name: name.to_string(),
                    installed,
                    path,
                    install: if installed {
                        None
                    } else {
                        Some(install_cmd.to_string())
                    },
                }
            });

            results.insert(
                lang.to_string(),
                LangStatus {
                    type_checker,
                    linter,
                },
            );
        }

        if writer.is_text() {
            let text = format_doctor_text(&results);
            writer.write_text(&text)?;
        } else {
            writer.write(&results)?;
        }

        Ok(())
    }
}

/// Format doctor results for human-readable text output
fn format_doctor_text(results: &BTreeMap<String, LangStatus>) -> String {
    use colored::Colorize;

    let mut output = String::new();

    // Header
    output.push_str(&"TLDR Diagnostics Check\n".bold().to_string());
    output.push_str("==================================================\n\n");

    let mut missing_count = 0;

    for (lang, status) in results {
        let mut lines: Vec<String> = Vec::new();

        if let Some(tc) = &status.type_checker {
            if tc.installed {
                lines.push(format!(
                    "  {} {} - {}",
                    "[OK]".green(),
                    tc.name,
                    tc.path.as_deref().unwrap_or("unknown")
                ));
            } else {
                lines.push(format!("  {} {} - not found", "[X]".red(), tc.name));
                if let Some(install) = &tc.install {
                    lines.push(format!("    -> {}", install));
                }
                missing_count += 1;
            }
        }

        if let Some(linter) = &status.linter {
            if linter.installed {
                lines.push(format!(
                    "  {} {} - {}",
                    "[OK]".green(),
                    linter.name,
                    linter.path.as_deref().unwrap_or("unknown")
                ));
            } else {
                lines.push(format!("  {} {} - not found", "[X]".red(), linter.name));
                if let Some(install) = &linter.install {
                    lines.push(format!("    -> {}", install));
                }
                missing_count += 1;
            }
        }

        if !lines.is_empty() {
            // Capitalize the language name for display
            let display_name = format!(
                "{}{}:",
                lang.chars().next().unwrap().to_uppercase(),
                &lang[1..]
            );
            output.push_str(&display_name);
            output.push('\n');
            for line in lines {
                output.push_str(&line);
                output.push('\n');
            }
            output.push('\n');
        }
    }

    if missing_count > 0 {
        output.push_str(&format!(
            "Missing {} tool(s). Run: tldr doctor --install <lang>\n",
            missing_count
        ));
    } else {
        output.push_str("All diagnostic tools installed!\n");
    }

    output
}
