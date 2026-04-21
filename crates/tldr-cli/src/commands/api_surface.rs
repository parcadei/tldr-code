//! API Surface command - Extract machine-readable API surface (structural contracts).
//!
//! Extracts the complete public API surface of a library or package, including:
//! - Function and method signatures with typed parameters
//! - Class definitions with constructor signatures
//! - Constants and type aliases
//! - Usage examples (templated from types)
//! - Trigger keywords for intent-based retrieval
//!
//! # Usage
//! ```bash
//! # Extract API surface for an installed Python package
//! tldr surface json --lang python --format json
//!
//! # Extract from a directory
//! tldr surface ./src/mylib/ --format text
//!
//! # Look up a specific API
//! tldr surface json --lookup json.loads --format text
//! ```

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::surface::{extract_api_surface, format_api_surface_text};
use tldr_core::Language;

use crate::output::{OutputFormat, OutputWriter};

/// Extract machine-readable API surface (structural contracts) for a library/package
#[derive(Debug, Args)]
pub struct ApiSurfaceArgs {
    /// Package name (e.g., "json", "flask") or directory path
    pub target: String,

    /// Lookup a specific API by qualified name (e.g., "json.loads")
    #[arg(long)]
    pub lookup: Option<String>,

    /// Include private/internal APIs (default: public only)
    #[arg(long)]
    pub include_private: bool,

    /// Maximum APIs to extract (default: unlimited)
    #[arg(long)]
    pub limit: Option<usize>,

    /// Path to Cargo.toml for Rust crate resolution
    #[arg(long)]
    pub manifest_path: Option<PathBuf>,
}

impl ApiSurfaceArgs {
    /// Run the API surface extraction command.
    ///
    /// The language is provided by the global `--lang` flag from CLI.
    pub fn run(&self, format: OutputFormat, quiet: bool, lang: Option<Language>) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        writer.progress(&format!(
            "Extracting API surface for '{}'...",
            self.target
        ));

        // Convert Language enum to string for the surface module
        let lang_str = lang.map(language_to_string);

        // If --manifest-path is provided and lang is Rust, use the parent directory
        // of the manifest as the target for crate resolution.
        let effective_target = if let Some(ref manifest) = self.manifest_path {
            let is_rust = lang_str.as_deref() == Some("rust");
            if is_rust {
                manifest
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| self.target.clone())
            } else {
                self.target.clone()
            }
        } else {
            self.target.clone()
        };

        let surface = extract_api_surface(
            &effective_target,
            lang_str.as_deref(),
            self.include_private,
            self.limit,
            self.lookup.as_deref(),
        )?;

        if writer.is_text() {
            writer.write_text(&format_api_surface_text(&surface))?;
        } else {
            writer.write(&surface)?;
        }

        Ok(())
    }
}

/// Convert a Language enum to the string expected by the contracts module.
fn language_to_string(lang: Language) -> String {
    match lang {
        Language::Python => "python".to_string(),
        Language::TypeScript => "typescript".to_string(),
        Language::JavaScript => "javascript".to_string(),
        Language::Go => "go".to_string(),
        Language::Rust => "rust".to_string(),
        Language::Java => "java".to_string(),
        Language::C => "c".to_string(),
        Language::Cpp => "cpp".to_string(),
        Language::Ruby => "ruby".to_string(),
        Language::Kotlin => "kotlin".to_string(),
        Language::Swift => "swift".to_string(),
        Language::CSharp => "csharp".to_string(),
        Language::Scala => "scala".to_string(),
        Language::Php => "php".to_string(),
        Language::Lua => "lua".to_string(),
        Language::Luau => "luau".to_string(),
        Language::Elixir => "elixir".to_string(),
        Language::Ocaml => "ocaml".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_api_surface_args_has_manifest_path_field() {
        let args = ApiSurfaceArgs {
            target: "my-crate".to_string(),
            lookup: None,
            include_private: false,
            limit: None,
            manifest_path: Some(PathBuf::from("./Cargo.toml")),
        };
        assert_eq!(
            args.manifest_path,
            Some(PathBuf::from("./Cargo.toml"))
        );
    }

    #[test]
    fn test_api_surface_args_manifest_path_defaults_to_none() {
        let args = ApiSurfaceArgs {
            target: "json".to_string(),
            lookup: None,
            include_private: false,
            limit: None,
            manifest_path: None,
        };
        assert!(args.manifest_path.is_none());
    }
}
