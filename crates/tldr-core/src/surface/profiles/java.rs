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
const PREFIX_MAIN_JAVA: &[&str] = &["src", "main", "java"];
const PREFIX_MAIN_RESOURCES: &[&str] = &["src", "main", "resources"];
const PREFIX_SRC: &[&str] = &["src"];
const DROP_PREFIXES: &[&[&str]] = &[PREFIX_MAIN_JAVA, PREFIX_MAIN_RESOURCES, PREFIX_SRC];
const PREFERRED_ROOTS: &[&str] = &["src/main/java"];
const ENTRYPOINTS: &[&str] = &[
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "src/main/java",
];

pub(super) const PROFILE: SurfaceLanguageProfile = SurfaceLanguageProfile {
    language: Language::Java,
    noise_dirs: NOISE_DIRS,
    noise_file_suffixes: NOISE_FILE_SUFFIXES,
    drop_segments: DROP_SEGMENTS,
    drop_prefixes: DROP_PREFIXES,
    preferred_roots: PREFERRED_ROOTS,
    entrypoints: ENTRYPOINTS,
};
