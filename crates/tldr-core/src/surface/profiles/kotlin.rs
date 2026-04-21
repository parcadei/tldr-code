use crate::types::Language;

use super::super::SurfaceLanguageProfile;

const NOISE_DIRS: &[&str] = &[
    "benchmark",
    "benchmarks",
    "doc",
    "docs",
    "example",
    "examples",
    "sample",
    "samples",
    "test",
    "tests",
];
const NOISE_FILE_SUFFIXES: &[&str] = &[];
const DROP_SEGMENTS: &[&str] = &["src"];
const PREFIX_MAIN_KOTLIN: &[&str] = &["src", "main", "kotlin"];
const PREFIX_COMMON_KOTLIN: &[&str] = &["src", "commonMain", "kotlin"];
const PREFIX_JVM_KOTLIN: &[&str] = &["src", "jvmMain", "kotlin"];
const PREFIX_NATIVE_KOTLIN: &[&str] = &["src", "nativeMain", "kotlin"];
const PREFIX_SRC: &[&str] = &["src"];
const DROP_PREFIXES: &[&[&str]] = &[
    PREFIX_MAIN_KOTLIN,
    PREFIX_COMMON_KOTLIN,
    PREFIX_JVM_KOTLIN,
    PREFIX_NATIVE_KOTLIN,
    PREFIX_SRC,
];
const PREFERRED_ROOTS: &[&str] = &["src/main/kotlin", "src/commonMain/kotlin"];
const ENTRYPOINTS: &[&str] = &[
    "build.gradle.kts",
    "build.gradle",
    "settings.gradle.kts",
    "src/main/kotlin",
    "src/commonMain/kotlin",
];

pub(super) const PROFILE: SurfaceLanguageProfile = SurfaceLanguageProfile {
    language: Language::Kotlin,
    noise_dirs: NOISE_DIRS,
    noise_file_suffixes: NOISE_FILE_SUFFIXES,
    drop_segments: DROP_SEGMENTS,
    drop_prefixes: DROP_PREFIXES,
    preferred_roots: PREFERRED_ROOTS,
    entrypoints: ENTRYPOINTS,
};
