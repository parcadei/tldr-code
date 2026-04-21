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
const DROP_SEGMENTS: &[&str] = &["sources"];
const PREFIX_SOURCES: &[&str] = &["Sources"];
const DROP_PREFIXES: &[&[&str]] = &[PREFIX_SOURCES];
const PREFERRED_ROOTS: &[&str] = &["Sources"];
const ENTRYPOINTS: &[&str] = &["Package.swift", "Sources"];

pub(super) const PROFILE: SurfaceLanguageProfile = SurfaceLanguageProfile {
    language: Language::Swift,
    noise_dirs: NOISE_DIRS,
    noise_file_suffixes: NOISE_FILE_SUFFIXES,
    drop_segments: DROP_SEGMENTS,
    drop_prefixes: DROP_PREFIXES,
    preferred_roots: PREFERRED_ROOTS,
    entrypoints: ENTRYPOINTS,
};
