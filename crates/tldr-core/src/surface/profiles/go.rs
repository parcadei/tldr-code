use crate::types::Language;

use super::super::SurfaceLanguageProfile;

const NOISE_DIRS: &[&str] = &[
    "benchmark",
    "benchmarks",
    "doc",
    "docs",
    "example",
    "examples",
    "testdata",
];
const NOISE_FILE_SUFFIXES: &[&str] = &["_benchmark.go", "_test.go"];
const DROP_SEGMENTS: &[&str] = &[];
const DROP_PREFIXES: &[&[&str]] = &[];
const PREFERRED_ROOTS: &[&str] = &["."];
const ENTRYPOINTS: &[&str] = &["go.mod"];

pub(super) const PROFILE: SurfaceLanguageProfile = SurfaceLanguageProfile {
    language: Language::Go,
    noise_dirs: NOISE_DIRS,
    noise_file_suffixes: NOISE_FILE_SUFFIXES,
    drop_segments: DROP_SEGMENTS,
    drop_prefixes: DROP_PREFIXES,
    preferred_roots: PREFERRED_ROOTS,
    entrypoints: ENTRYPOINTS,
};
