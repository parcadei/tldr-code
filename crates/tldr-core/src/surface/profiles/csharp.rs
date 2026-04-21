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
const DROP_SEGMENTS: &[&str] = &["source", "sources", "src"];
const PREFIX_SRC: &[&str] = &["src"];
const PREFIX_SOURCE: &[&str] = &["source"];
const PREFIX_SOURCES: &[&str] = &["sources"];
const DROP_PREFIXES: &[&[&str]] = &[PREFIX_SRC, PREFIX_SOURCE, PREFIX_SOURCES];
const PREFERRED_ROOTS: &[&str] = &["src", "source", "sources"];
const ENTRYPOINTS: &[&str] = &["src", ".csproj", "Directory.Build.props"];

pub(super) const PROFILE: SurfaceLanguageProfile = SurfaceLanguageProfile {
    language: Language::CSharp,
    noise_dirs: NOISE_DIRS,
    noise_file_suffixes: NOISE_FILE_SUFFIXES,
    drop_segments: DROP_SEGMENTS,
    drop_prefixes: DROP_PREFIXES,
    preferred_roots: PREFERRED_ROOTS,
    entrypoints: ENTRYPOINTS,
};
