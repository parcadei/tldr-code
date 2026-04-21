//! Per-language static surface profiles.

use crate::types::Language;

use super::SurfaceLanguageProfile;

pub(super) mod c;
pub(super) mod cpp;
pub(super) mod csharp;
pub(super) mod elixir;
pub(super) mod go;
pub(super) mod java;
pub(super) mod javascript;
pub(super) mod kotlin;
pub(super) mod lua;
pub(super) mod php;
pub(super) mod python;
pub(super) mod ruby;
pub(super) mod rust;
pub(super) mod scala;
pub(super) mod swift;
pub(super) mod typescript;

/// Languages with dedicated static surface profiles.
pub(crate) const SUPPORTED_SURFACE_LANGUAGES: &[Language] = &[
    Language::C,
    Language::Cpp,
    Language::CSharp,
    Language::Elixir,
    Language::Go,
    Language::Java,
    Language::JavaScript,
    Language::Kotlin,
    Language::Lua,
    Language::Php,
    Language::Python,
    Language::Ruby,
    Language::Rust,
    Language::Scala,
    Language::Swift,
    Language::TypeScript,
];

/// Return the static surface profile for a supported language.
#[must_use]
pub(crate) fn profile_for(language: Language) -> Option<&'static SurfaceLanguageProfile> {
    match language {
        Language::C => Some(&c::PROFILE),
        Language::Cpp => Some(&cpp::PROFILE),
        Language::CSharp => Some(&csharp::PROFILE),
        Language::Elixir => Some(&elixir::PROFILE),
        Language::Go => Some(&go::PROFILE),
        Language::Java => Some(&java::PROFILE),
        Language::JavaScript => Some(&javascript::PROFILE),
        Language::Kotlin => Some(&kotlin::PROFILE),
        Language::Lua => Some(&lua::PROFILE),
        Language::Php => Some(&php::PROFILE),
        Language::Python => Some(&python::PROFILE),
        Language::Ruby => Some(&ruby::PROFILE),
        Language::Rust => Some(&rust::PROFILE),
        Language::Scala => Some(&scala::PROFILE),
        Language::Swift => Some(&swift::PROFILE),
        Language::TypeScript => Some(&typescript::PROFILE),
        Language::Luau | Language::Ocaml => None,
    }
}
