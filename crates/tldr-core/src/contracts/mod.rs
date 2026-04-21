//! Structural contracts: API surface extraction for libraries and packages.
//!
//! This module extracts machine-readable API surfaces from installed packages,
//! producing structured data about every public function, method, class, constant,
//! and type alias. The output includes:
//!
//! - Qualified names and module paths
//! - Typed signatures with parameter defaults
//! - Docstrings (first paragraph, truncated)
//! - Example usage strings (templated from types)
//! - Trigger keywords for intent-based retrieval
//!
//! # Relationship to behavioral contracts
//!
//! The CLI `contracts` command (in `tldr-cli/src/commands/contracts/`) extracts
//! *behavioral* contracts (pre/postconditions from guard clauses and assertions).
//! This module extracts *structural* contracts (API shapes from a library).
//! They are complementary, not overlapping.
//!
//! # Supported languages
//!
//! - **Python** (Phase 1): Full support via tree-sitter + C extension fallback
//! - Other languages planned for later phases

pub mod examples;
pub mod python;
pub mod resolve;
pub mod triggers;
pub mod types;

// Re-export core types for public API
pub use types::{ApiEntry, ApiKind, ApiSurface, Location, Param, Signature};

use crate::TldrResult;

/// Extract the complete API surface for a package.
///
/// This is the main entry point. It resolves the package path, determines the
/// language, and dispatches to the language-specific extractor.
///
/// # Arguments
/// * `target` - Package name (e.g., "flask") or directory path
/// * `lang` - Optional language hint (auto-detected if not specified)
/// * `include_private` - Whether to include private APIs
/// * `limit` - Optional maximum number of APIs
/// * `lookup` - Optional: return only the API matching this qualified name
///
/// # Returns
/// * `Ok(ApiSurface)` - The extracted API surface
/// * `Err(TldrError)` - If resolution or extraction fails
pub fn extract_api_surface(
    target: &str,
    lang: Option<&str>,
    include_private: bool,
    limit: Option<usize>,
    lookup: Option<&str>,
) -> TldrResult<ApiSurface> {
    // Resolve the target to a package directory
    let resolved = resolve::resolve_target(target, lang)?;

    // Determine language and dispatch
    let effective_lang = lang.unwrap_or("python");

    let mut surface = match effective_lang {
        "python" => python::extract_python_api_surface(&resolved, include_private, limit)?,
        other => {
            return Err(crate::error::TldrError::UnsupportedLanguage(format!(
                "API surface extraction not yet supported for language: {}",
                other
            )))
        }
    };

    // Handle --lookup: filter to a single API entry
    if let Some(lookup_name) = lookup {
        surface.apis.retain(|api| {
            api.qualified_name == lookup_name
                || api.qualified_name.ends_with(&format!(".{}", lookup_name))
        });
        surface.total = surface.apis.len();
    }

    Ok(surface)
}

/// Format an API surface as human-readable text.
pub fn format_api_surface_text(surface: &ApiSurface) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "API Surface: {} ({}) - {} APIs\n",
        surface.package, surface.language, surface.total
    ));
    output.push_str(&"=".repeat(60));
    output.push('\n');

    for api in &surface.apis {
        output.push('\n');

        // Kind and name
        output.push_str(&format!("[{}] {}\n", api.kind, api.qualified_name));

        // Signature
        if let Some(sig) = &api.signature {
            let params_str: Vec<String> = sig
                .params
                .iter()
                .map(|p| {
                    let mut s = p.name.clone();
                    if p.is_variadic {
                        s = format!("*{}", s);
                    }
                    if p.is_keyword {
                        s = format!("**{}", s);
                    }
                    if let Some(t) = &p.type_annotation {
                        s = format!("{}: {}", s, t);
                    }
                    if let Some(d) = &p.default {
                        s = format!("{} = {}", s, d);
                    }
                    s
                })
                .collect();

            let ret = sig
                .return_type
                .as_ref()
                .map(|r| format!(" -> {}", r))
                .unwrap_or_default();

            let async_prefix = if sig.is_async { "async " } else { "" };

            output.push_str(&format!(
                "  {}({}){}\n",
                async_prefix,
                params_str.join(", "),
                ret
            ));
        }

        // Docstring
        if let Some(doc) = &api.docstring {
            output.push_str(&format!("  {}\n", doc));
        }

        // Example
        if let Some(ex) = &api.example {
            output.push_str(&format!("  Example: {}\n", ex));
        }

        // Triggers
        if !api.triggers.is_empty() {
            output.push_str(&format!("  Triggers: {}\n", api.triggers.join(", ")));
        }

        // Location
        if let Some(loc) = &api.location {
            output.push_str(&format!("  Location: {}:{}\n", loc.file.display(), loc.line));
        }
    }

    output
}
