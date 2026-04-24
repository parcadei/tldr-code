//! Per-language static surface extraction profiles.
//!
//! This module isolates repository-shape and entrypoint heuristics for static
//! API surface extraction. The profiles are intentionally separate from the
//! extractors themselves so each language can evolve its own surface policy
//! without cross-language coupling.

use std::path::{Component, Path};

use crate::types::Language;

#[path = "profiles/mod.rs"]
mod profiles;

/// Static surface policy for one language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceLanguageProfile {
    /// Canonical language enum used by `surface`.
    pub language: Language,
    /// Exact directory names that should usually be ignored for API discovery.
    pub noise_dirs: &'static [&'static str],
    /// File suffixes that usually indicate tests, fixtures, or benchmarks.
    pub noise_file_suffixes: &'static [&'static str],
    /// Leading scaffolding segments to trim after prefix stripping.
    pub drop_segments: &'static [&'static str],
    /// Layout-specific multi-segment prefixes to trim from the start of a path.
    pub drop_prefixes: &'static [&'static [&'static str]],
    /// Preferred roots for package-facing API discovery.
    pub preferred_roots: &'static [&'static str],
    /// Files or roots that commonly define the package entrypoint.
    pub entrypoints: &'static [&'static str],
}

#[doc(hidden)]
pub trait SurfaceProfileLanguage {
    fn into_language(self) -> Option<Language>;
}

impl SurfaceProfileLanguage for Language {
    fn into_language(self) -> Option<Language> {
        Some(self)
    }
}

impl SurfaceProfileLanguage for &str {
    fn into_language(self) -> Option<Language> {
        match self {
            "c" => Some(Language::C),
            "cpp" => Some(Language::Cpp),
            "csharp" => Some(Language::CSharp),
            "elixir" => Some(Language::Elixir),
            "go" => Some(Language::Go),
            "java" => Some(Language::Java),
            "javascript" | "js" => Some(Language::JavaScript),
            "kotlin" => Some(Language::Kotlin),
            "lua" => Some(Language::Lua),
            "php" => Some(Language::Php),
            "python" => Some(Language::Python),
            "ruby" => Some(Language::Ruby),
            "rust" => Some(Language::Rust),
            "scala" => Some(Language::Scala),
            "swift" => Some(Language::Swift),
            "typescript" | "ts" => Some(Language::TypeScript),
            "luau" => Some(Language::Luau),
            "ocaml" => Some(Language::Ocaml),
            _ => None,
        }
    }
}

#[doc(hidden)]
pub trait IntoLayoutSegments {
    fn into_layout_segments(self) -> Vec<String>;
}

impl IntoLayoutSegments for &Path {
    fn into_layout_segments(self) -> Vec<String> {
        path_segments(self)
    }
}

impl IntoLayoutSegments for Vec<String> {
    fn into_layout_segments(self) -> Vec<String> {
        self
    }
}

/// Look up the static surface profile for one supported language.
#[must_use]
pub fn language_profile<L>(language: L) -> Option<&'static SurfaceLanguageProfile>
where
    L: SurfaceProfileLanguage,
{
    profiles::profile_for(language.into_language()?)
}

/// Return every language with a dedicated static surface profile.
#[must_use]
pub fn supported_surface_languages() -> &'static [Language] {
    profiles::SUPPORTED_SURFACE_LANGUAGES
}

/// Return `true` if a directory should usually be ignored for a language.
#[must_use]
pub fn is_noise_dir<L>(language: L, dir_name: &str) -> bool
where
    L: SurfaceProfileLanguage,
{
    let Some(profile) = language_profile(language) else {
        return false;
    };

    let candidate = dir_name.to_ascii_lowercase();
    profile.noise_dirs.iter().any(|entry| *entry == candidate)
}

/// Return `true` if a file name matches one of the language's noise suffixes.
#[must_use]
pub fn is_noise_file<L>(language: L, file_name: &str) -> bool
where
    L: SurfaceProfileLanguage,
{
    let Some(profile) = language_profile(language) else {
        return false;
    };

    let candidate = file_name.to_ascii_lowercase();
    profile
        .noise_file_suffixes
        .iter()
        .any(|suffix| candidate.ends_with(suffix))
}

/// Strip language-specific layout segments from a path.
///
/// This removes configured layout prefixes such as `src/main/java`,
/// `src/commonMain/kotlin`, or `Sources`, then trims any remaining leading
/// scaffolding segments like `src` or `lib`.
#[must_use]
pub fn strip_layout_segments<L, P>(language: L, path: P) -> Vec<String>
where
    L: SurfaceProfileLanguage,
    P: IntoLayoutSegments,
{
    let mut segments = path.into_layout_segments();
    let Some(profile) = language_profile(language) else {
        return segments;
    };
    if let Some((start, len)) = matching_prefix_len(&segments, profile.drop_prefixes) {
        segments.drain(start..start + len);
    }
    while segments
        .first()
        .is_some_and(|segment| contains_ignore_case(profile.drop_segments, segment))
    {
        segments.remove(0);
    }
    segments
}

/// Return the canonical entrypoint candidates for a language profile.
#[must_use]
pub fn entrypoint_candidates<L>(language: L) -> &'static [&'static str]
where
    L: SurfaceProfileLanguage,
{
    language_profile(language)
        .map(|profile| profile.entrypoints)
        .unwrap_or(&[])
}

/// Compute a static preference score for a relative path.
///
/// This is a small shared ranking scaffold for future extractor integration.
/// Higher scores indicate paths that look more package-facing according to the
/// language profile:
///
/// - entrypoints score highest
/// - preferred roots receive a smaller positive boost
/// - neutral paths stay at `0`
/// - paths containing known noise directories are demoted
#[must_use]
pub fn static_preference_score<L>(language: L, relative_path: &Path) -> i32
where
    L: SurfaceProfileLanguage,
{
    let Some(profile) = language_profile(language) else {
        return 0;
    };

    let segments = path_segments(relative_path);
    if segments.is_empty() {
        return 0;
    }

    if segments
        .iter()
        .any(|segment| contains_ignore_case(profile.noise_dirs, segment))
    {
        return -100;
    }

    let mut score = 0;

    if profile
        .entrypoints
        .iter()
        .any(|rule| path_matches_rule(&segments, rule))
    {
        score += 100;
    }

    if profile
        .preferred_roots
        .iter()
        .any(|rule| path_matches_rule(&segments, rule))
    {
        score += 25;
    }

    score
}

fn path_segments(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(segment) => Some(segment.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn matching_prefix_len(
    segments: &[String],
    prefixes: &'static [&'static [&'static str]],
) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None; // (start, prefix_len)
    for prefix in prefixes {
        for start in 0..segments.len() {
            if starts_with_ignore_case(&segments[start..], prefix)
                && best.is_none_or(|(_, prev_len)| prefix.len() > prev_len)
            {
                best = Some((start, prefix.len()));
            }
        }
    }
    best
}

fn starts_with_ignore_case(segments: &[String], prefix: &[&str]) -> bool {
    segments.len() >= prefix.len()
        && segments
            .iter()
            .take(prefix.len())
            .zip(prefix.iter())
            .all(|(segment, expected)| segment.eq_ignore_ascii_case(expected))
}

fn contains_ignore_case(values: &[&str], candidate: &str) -> bool {
    values
        .iter()
        .any(|value| candidate.eq_ignore_ascii_case(value))
}

fn path_matches_rule(segments: &[String], rule: &str) -> bool {
    if rule == "." {
        return true;
    }

    let rule_segments: Vec<&str> = rule
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();

    if rule_segments.is_empty() {
        return false;
    }

    if starts_with_ignore_case(segments, &rule_segments) {
        return true;
    }

    rule_segments.len() == 1
        && segments
            .last()
            .is_some_and(|segment| segment.eq_ignore_ascii_case(rule_segments[0]))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::types::Language;

    use super::{
        entrypoint_candidates, is_noise_dir, is_noise_file, language_profile,
        static_preference_score, strip_layout_segments, supported_surface_languages,
    };

    #[test]
    fn profile_lookup_returns_supported_profile() {
        let profile = language_profile(Language::Rust).expect("rust profile should exist");

        assert_eq!(profile.language, Language::Rust);
        assert!(profile.preferred_roots.contains(&"src"));
    }

    #[test]
    fn profile_lookup_excludes_unsupported_languages() {
        assert!(language_profile(Language::Luau).is_none());
        assert!(language_profile(Language::Ocaml).is_none());
    }

    #[test]
    fn supported_languages_are_surface_scoped() {
        let supported = supported_surface_languages();

        assert!(supported.contains(&Language::Python));
        assert!(supported.contains(&Language::TypeScript));
        assert!(supported.contains(&Language::Swift));
        assert!(!supported.contains(&Language::Luau));
        assert!(!supported.contains(&Language::Ocaml));
        assert_eq!(supported.len(), 16);
    }

    #[test]
    fn helper_behavior_uses_profile_rules() {
        assert!(is_noise_dir(Language::Python, "docs_src"));
        assert!(is_noise_dir(Language::JavaScript, "__fixtures__"));
        assert!(is_noise_file(Language::JavaScript, "widget.spec.js"));
        assert!(is_noise_file(Language::TypeScript, "widget.benchmark.tsx"));
        assert!(!is_noise_file(Language::TypeScript, "index.d.ts"));
    }

    #[test]
    fn layout_stripping_removes_language_specific_prefixes() {
        let java_segments = strip_layout_segments(
            Language::Java,
            Path::new("src/main/java/com/example/api/Foo.java"),
        );
        let kotlin_segments = strip_layout_segments(
            Language::Kotlin,
            Path::new("src/commonMain/kotlin/com/example/api/Foo.kt"),
        );
        let swift_segments = strip_layout_segments(
            Language::Swift,
            Path::new("Sources/Networking/Client.swift"),
        );

        assert_eq!(java_segments, ["com", "example", "api", "Foo.java"]);
        assert_eq!(kotlin_segments, ["com", "example", "api", "Foo.kt"]);
        assert_eq!(swift_segments, ["Networking", "Client.swift"]);
    }

    #[test]
    fn layout_stripping_leaves_unsupported_languages_unchanged() {
        let segments = strip_layout_segments(Language::Luau, Path::new("src/pkg/init.luau"));

        assert_eq!(segments, ["src", "pkg", "init.luau"]);
    }

    #[test]
    fn entrypoint_candidates_come_from_profile_data() {
        let rust_entrypoints = entrypoint_candidates(Language::Rust);
        let python_entrypoints = entrypoint_candidates(Language::Python);

        assert!(rust_entrypoints.contains(&"src/lib.rs"));
        assert!(python_entrypoints.contains(&"pyproject.toml"));
        assert!(entrypoint_candidates(Language::Luau).is_empty());
    }

    #[test]
    fn static_preference_score_prioritizes_entrypoints_over_preferred_roots() {
        let entrypoint = static_preference_score(Language::TypeScript, Path::new("src/index.ts"));
        let preferred_root =
            static_preference_score(Language::TypeScript, Path::new("src/components/button.ts"));

        assert!(entrypoint > preferred_root);
    }

    #[test]
    fn static_preference_score_prioritizes_preferred_roots_over_neutral_paths() {
        let preferred_root = static_preference_score(Language::Rust, Path::new("src/lib.rs"));
        let neutral = static_preference_score(Language::Rust, Path::new("internal/helpers/mod.rs"));

        assert!(preferred_root > neutral);
    }

    #[test]
    fn static_preference_score_demotes_noise_paths_below_neutral_paths() {
        let neutral = static_preference_score(Language::Rust, Path::new("internal/helpers/mod.rs"));
        let noise = static_preference_score(Language::Rust, Path::new("examples/demo/main.rs"));

        assert!(neutral > noise);
    }

    #[test]
    fn static_preference_score_orders_entrypoint_preferred_neutral_and_noise_paths() {
        let entrypoint = static_preference_score(Language::TypeScript, Path::new("src/index.ts"));
        let preferred_root =
            static_preference_score(Language::TypeScript, Path::new("src/components/button.ts"));
        let neutral =
            static_preference_score(Language::TypeScript, Path::new("scripts/release.ts"));
        let noise =
            static_preference_score(Language::TypeScript, Path::new("examples/demo/index.ts"));

        assert!(entrypoint > preferred_root);
        assert!(preferred_root > neutral);
        assert!(neutral > noise);
    }
}
