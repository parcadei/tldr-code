use crate::types::Language;

use super::super::SurfaceLanguageProfile;

const NOISE_DIRS: &[&str] = &[
    "bench",
    "benches",
    "benchmark",
    "benchmarks",
    "doc",
    "docs",
    "example",
    "examples",
    "test",
    "tests",
    "third_party",
    "vendor",
];
const NOISE_FILE_SUFFIXES: &[&str] = &[
    "_bench.c",
    "_benchmark.c",
    "_fixture.c",
    "_test.c",
    "_tests.c",
];
const DROP_SEGMENTS: &[&str] = &["include", "src"];
const PREFIX_INCLUDE: &[&str] = &["include"];
const PREFIX_SRC: &[&str] = &["src"];
const DROP_PREFIXES: &[&[&str]] = &[PREFIX_INCLUDE, PREFIX_SRC];
const PREFERRED_ROOTS: &[&str] = &["include", "src"];
const ENTRYPOINTS: &[&str] = &["include", "src", "meson.build", "CMakeLists.txt"];

pub(super) const PROFILE: SurfaceLanguageProfile = SurfaceLanguageProfile {
    language: Language::C,
    noise_dirs: NOISE_DIRS,
    noise_file_suffixes: NOISE_FILE_SUFFIXES,
    drop_segments: DROP_SEGMENTS,
    drop_prefixes: DROP_PREFIXES,
    preferred_roots: PREFERRED_ROOTS,
    entrypoints: ENTRYPOINTS,
};
