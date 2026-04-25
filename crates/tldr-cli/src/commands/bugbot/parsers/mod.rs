//! Output parsers for commodity diagnostic tools
//!
//! Each parser converts tool-specific output formats (NDJSON, JSON, etc.)
//! into a uniform `Vec<L1Finding>`.

pub mod cargo;
pub mod cargo_audit;
pub mod checkstyle;
pub mod cppcheck;
pub mod eslint;
pub mod golangci_lint;
pub mod ktlint;
pub mod luacheck;
pub mod phpstan;
pub mod pyright;
pub mod rubocop;
pub mod ruff;
pub mod swiftlint;

use super::tools::L1Finding;

/// Errors that can occur during output parsing
#[derive(Debug)]
pub enum ParseError {
    /// JSON parsing failed
    Json(serde_json::Error),
    /// Output format not recognized
    Format(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Json(e) => write!(f, "JSON parse error: {}", e),
            ParseError::Format(s) => write!(f, "Format error: {}", s),
        }
    }
}

impl std::error::Error for ParseError {}

impl From<serde_json::Error> for ParseError {
    fn from(e: serde_json::Error) -> Self {
        ParseError::Json(e)
    }
}

/// Parse tool output using the named parser.
///
/// Dispatches to the appropriate parser based on `parser_name`.
/// Returns `ParseError::Format` for unknown parser names.
pub fn parse_tool_output(parser_name: &str, stdout: &str) -> Result<Vec<L1Finding>, ParseError> {
    match parser_name {
        "cargo" => Ok(cargo::parse_cargo_output(stdout)),
        "cargo-audit" => cargo_audit::parse_cargo_audit_output(stdout),
        "checkstyle" => Ok(checkstyle::parse_checkstyle_output(stdout)?),
        "cppcheck" => Ok(cppcheck::parse_cppcheck_output(stdout)?),
        "eslint" => eslint::parse_eslint_output(stdout),
        "golangci-lint" => golangci_lint::parse_golangci_lint_output(stdout),
        "ktlint" => ktlint::parse_ktlint_output(stdout),
        "luacheck" => Ok(luacheck::parse_luacheck_output(stdout)?),
        "phpstan" => phpstan::parse_phpstan_output(stdout),
        "pyright" => pyright::parse_pyright_output(stdout),
        "rubocop" => rubocop::parse_rubocop_output(stdout),
        "ruff" => ruff::parse_ruff_output(stdout),
        "swiftlint" => swiftlint::parse_swiftlint_output(stdout),
        _ => Err(ParseError::Format(format!(
            "Unknown parser: {}",
            parser_name
        ))),
    }
}
