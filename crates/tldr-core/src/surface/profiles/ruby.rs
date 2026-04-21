use crate::types::Language;

use super::super::SurfaceLanguageProfile;

const NOISE_DIRS: &[&str] = &[
    "benchmark",
    "benchmarks",
    "doc",
    "docs",
    "example",
    "examples",
    "spec",
    "specs",
    "test",
    "tests",
];
const NOISE_FILE_SUFFIXES: &[&str] = &["_benchmark.rb", "_bench.rb", "_spec.rb", "_test.rb"];
const DROP_SEGMENTS: &[&str] = &["lib"];
const PREFIX_LIB: &[&str] = &["lib"];
const DROP_PREFIXES: &[&[&str]] = &[PREFIX_LIB];
const PREFERRED_ROOTS: &[&str] = &["lib"];
const ENTRYPOINTS: &[&str] = &[".gemspec", "Gemfile", "lib"];

pub(super) const PROFILE: SurfaceLanguageProfile = SurfaceLanguageProfile {
    language: Language::Ruby,
    noise_dirs: NOISE_DIRS,
    noise_file_suffixes: NOISE_FILE_SUFFIXES,
    drop_segments: DROP_SEGMENTS,
    drop_prefixes: DROP_PREFIXES,
    preferred_roots: PREFERRED_ROOTS,
    entrypoints: ENTRYPOINTS,
};
