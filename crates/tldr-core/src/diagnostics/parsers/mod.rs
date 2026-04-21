//! Parser modules for various diagnostic tool outputs.
//!
//! Each parser converts tool-specific output format into unified `Diagnostic` structs.
//!
//! # Supported Tools
//!
//! ## Python
//! - `pyright`: Type checker with JSON output (`--outputjson`)
//! - `ruff`: Fast linter with JSON output (`--output-format json`)
//!
//! ## TypeScript/JavaScript
//! - `tsc`: TypeScript compiler (text output, parsed via regex)
//! - `eslint`: Linter with JSON output (`-f json`)
//!
//! ## Rust
//! - `cargo`: Compiler with JSON output (`--message-format=json`)
//! - `clippy`: Linter using same format as cargo
//!
//! ## Go
//! - `go vet`: Static analysis with JSON output (`-json`)
//! - `golangci-lint`: Meta-linter with JSON output (`--out-format json`)
//!
//! ## Kotlin
//! - `kotlinc`: Compiler with GCC-like text output
//! - `detekt`: Static analysis with text output
//!
//! ## Swift
//! - `swiftc`: Compiler with GCC-like text output
//! - `swiftlint`: Linter with JSON output (`--reporter json`)
//!
//! ## C#
//! - `dotnet build`: Compiler/analyzer with MSBuild text output
//!
//! ## Scala
//! - `scalac`: Compiler with GCC-like text output
//!
//! ## Elixir
//! - `mix compile`: Compiler with text output
//! - `credo`: Static analysis with JSON output (`--format json`)
//!
//! ## Lua
//! - `luacheck`: Linter with plain text output (`--formatter plain`)
//!
//! ## Java
//! - `javac`: Compiler with text output
//! - `checkstyle`: Style checker with plain text output (`-f plain`)
//!
//! ## C/C++
//! - `clang`/`gcc`: Compiler with GCC-style text output (`-fsyntax-only -Wall`)
//! - `clang-tidy`: Static analysis with GCC-style text output
//!
//! ## Ruby
//! - `rubocop`: Linter with JSON output (`--format json`)
//!
//! ## PHP
//! - `php -l`: Syntax checker with text output
//! - `phpstan`: Static analysis with JSON output (`--error-format=json`)

mod cargo;
pub mod clang;
pub mod csharp;
pub mod elixir;
mod eslint;
mod go;
pub mod java;
pub mod kotlin;
pub mod lua;
pub mod php;
mod pyright;
pub mod ruby;
mod ruff;
pub mod scala;
pub mod swift;
mod tsc;

pub use cargo::parse_cargo_output;
pub use clang::parse_clang_output;
pub use csharp::parse_dotnet_build_output;
pub use elixir::{parse_credo_output, parse_mix_compile_output};
pub use eslint::parse_eslint_output;
pub use go::{parse_go_vet_output, parse_golangci_lint_output};
pub use java::{parse_checkstyle_output, parse_javac_output};
pub use kotlin::{parse_detekt_output, parse_kotlinc_output};
pub use lua::parse_luacheck_output;
pub use php::{parse_php_lint_output, parse_phpstan_output};
pub use pyright::parse_pyright_output;
pub use ruby::parse_rubocop_output;
pub use ruff::parse_ruff_output;
pub use scala::parse_scalac_output;
pub use swift::{parse_swiftc_output, parse_swiftlint_output};
pub use tsc::{parse_tsc_text, tsc_output_regex};

use crate::diagnostics::Severity;

/// Map a string severity to our Severity enum.
/// Used by multiple parsers with similar severity naming.
pub fn map_severity(s: &str) -> Severity {
    match s.to_lowercase().as_str() {
        "error" => Severity::Error,
        "warning" | "warn" => Severity::Warning,
        "information" | "info" | "note" => Severity::Information,
        "hint" | "suggestion" => Severity::Hint,
        _ => Severity::Warning, // Default to warning for unknown
    }
}
