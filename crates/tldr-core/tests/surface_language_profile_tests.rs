use std::collections::HashSet;

use tldr_core::surface::extract_api_surface;
use std::path::Path;

use tldr_core::surface::language_profile::{
    language_profile, static_preference_score, supported_surface_languages,
};
use tldr_core::types::Language;

#[path = "support/surface_language_profiles.rs"]
mod support;

use support::{all_surface_language_profiles, has_empty_path_segment, materialize_case};

#[test]
fn test_every_supported_surface_language_has_a_validation_profile() {
    let actual: HashSet<Language> = all_surface_language_profiles()
        .iter()
        .map(|case| case.language)
        .collect();
    let expected: HashSet<Language> = supported_surface_languages().iter().copied().collect();

    assert_eq!(actual, expected);

    for language in supported_surface_languages() {
        assert!(
            language_profile(*language).is_some(),
            "missing surface language profile for {:?}",
            language
        );
    }
}

#[test]
fn test_surface_language_profile_invariants_hold() {
    let mut langs = HashSet::new();

    for language in supported_surface_languages() {
        let profile = language_profile(*language)
            .unwrap_or_else(|| panic!("missing profile for {:?}", language));

        assert!(
            langs.insert(profile.language),
            "duplicate profile for language {:?}",
            profile.language
        );

        let entrypoints: HashSet<&str> = profile.entrypoints.iter().copied().collect();
        assert_eq!(
            entrypoints.len(),
            profile.entrypoints.len(),
            "language {:?} has duplicate entrypoints",
            profile.language
        );
        for entrypoint in profile.entrypoints {
            assert!(
                !entrypoint.trim().is_empty(),
                "language {:?} has empty entrypoint",
                profile.language
            );
            assert!(
                !has_empty_path_segment(entrypoint),
                "language {:?} has entrypoint with empty path segment: {}",
                profile.language,
                entrypoint
            );
        }

        let noise_dirs: HashSet<&str> = profile.noise_dirs.iter().copied().collect();
        assert_eq!(
            noise_dirs.len(),
            profile.noise_dirs.len(),
            "language {:?} has duplicate noise dirs",
            profile.language
        );
        for dir in profile.noise_dirs {
            assert!(
                !dir.trim().is_empty(),
                "language {:?} has empty noise dir",
                profile.language
            );
            assert!(
                !dir.contains('/'),
                "language {:?} noise dir must be a single segment: {}",
                profile.language,
                dir
            );
        }

        let noise_suffixes: HashSet<&str> = profile.noise_file_suffixes.iter().copied().collect();
        assert_eq!(
            noise_suffixes.len(),
            profile.noise_file_suffixes.len(),
            "language {:?} has duplicate noise suffixes",
            profile.language
        );
        for suffix in profile.noise_file_suffixes {
            assert!(
                !suffix.trim().is_empty(),
                "language {:?} has empty noise suffix",
                profile.language
            );
            assert!(
                !suffix.contains('/'),
                "language {:?} noise suffix must not contain path separators: {}",
                profile.language,
                suffix
            );
        }
    }
}

#[test]
fn test_static_preference_score_orders_paths_by_surface_signal() {
    let entrypoint = static_preference_score(Language::TypeScript, Path::new("src/index.ts"));
    let preferred_root =
        static_preference_score(Language::TypeScript, Path::new("src/components/button.ts"));
    let neutral = static_preference_score(Language::TypeScript, Path::new("scripts/release.ts"));
    let noise = static_preference_score(Language::TypeScript, Path::new("examples/demo/index.ts"));

    assert!(entrypoint > preferred_root);
    assert!(preferred_root > neutral);
    assert!(neutral > noise);
}

#[test]
fn test_surface_language_smoke_matrix_extracts_expected_public_api() {
    for case in all_surface_language_profiles() {
        let (_temp_dir, target_path) = materialize_case(case);
        let surface = extract_api_surface(
            target_path.to_str().expect("target path utf-8"),
            Some(case.language.as_str()),
            false,
            None,
            None,
        )
        .unwrap_or_else(|err| panic!("surface extraction failed for {:?}: {err}", case.language));

        assert_eq!(
            surface.language,
            case.language.as_str(),
            "wrong language for {:?}",
            case.language
        );
        assert_eq!(
            surface.total,
            surface.apis.len(),
            "surface total mismatch for {:?}",
            case.language
        );
        assert!(
            !surface.apis.is_empty(),
            "expected at least one API for {:?}",
            case.language
        );

        let qualified_names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        for expected in case.expected_public_symbols {
            assert!(
                qualified_names.iter().any(|name| name.contains(expected)),
                "language {:?} missing expected symbol {} in {:?}",
                case.language,
                expected,
                qualified_names
            );
        }

        for excluded in case.excluded_noise_symbols {
            assert!(
                !qualified_names.iter().any(|name| name.contains(excluded)),
                "language {:?} unexpectedly surfaced noise symbol {} in {:?}",
                case.language,
                excluded,
                qualified_names
            );
        }
    }
}

#[test]
fn test_surface_language_smoke_matrix_preserves_basic_output_invariants() {
    for case in all_surface_language_profiles() {
        let (_temp_dir, target_path) = materialize_case(case);
        let surface = extract_api_surface(
            target_path.to_str().expect("target path utf-8"),
            Some(case.language.as_str()),
            false,
            None,
            None,
        )
        .unwrap_or_else(|err| panic!("surface extraction failed for {:?}: {err}", case.language));

        let mut seen_qualified_names = HashSet::new();
        for api in &surface.apis {
            assert!(
                !api.qualified_name.trim().is_empty(),
                "empty qualified name in {:?}",
                case.language
            );
            assert!(
                !api.qualified_name.contains(".."),
                "empty qualified name segment in {:?}: {}",
                case.language,
                api.qualified_name
            );
            assert!(
                seen_qualified_names.insert(api.qualified_name.as_str()),
                "duplicate qualified name in {:?}: {}",
                case.language,
                api.qualified_name
            );

            if let Some(location) = &api.location {
                assert!(
                    !location.file.is_absolute(),
                    "absolute location path in {:?}: {:?}",
                    case.language,
                    location.file
                );
            }
        }
    }
}
