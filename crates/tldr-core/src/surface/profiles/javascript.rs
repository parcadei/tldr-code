use crate::types::Language;

use super::super::SurfaceLanguageProfile;

const NOISE_DIRS: &[&str] = &[
    "__fixtures__",
    "__tests__",
    "bench",
    "benches",
    "benchmark",
    "benchmarks",
    "doc",
    "docs",
    "example",
    "examples",
    "fixture",
    "fixtures",
    "spec",
    "specs",
    "test",
    "tests",
];
const NOISE_FILE_SUFFIXES: &[&str] = &[
    ".bench.cjs",
    ".bench.js",
    ".bench.mjs",
    ".benchmark.cjs",
    ".benchmark.js",
    ".benchmark.mjs",
    ".fixture.cjs",
    ".fixture.js",
    ".fixture.mjs",
    ".spec.cjs",
    ".spec.js",
    ".spec.mjs",
    ".test.cjs",
    ".test.js",
    ".test.mjs",
];
const DROP_SEGMENTS: &[&str] = &["cjs", "dist", "esm", "index", "lib", "src"];
const PREFIX_DIST: &[&str] = &["dist"];
const PREFIX_ESM: &[&str] = &["esm"];
const PREFIX_CJS: &[&str] = &["cjs"];
const PREFIX_LIB: &[&str] = &["lib"];
const PREFIX_SRC: &[&str] = &["src"];
const DROP_PREFIXES: &[&[&str]] = &[PREFIX_SRC, PREFIX_LIB, PREFIX_DIST, PREFIX_ESM, PREFIX_CJS];
const PREFERRED_ROOTS: &[&str] = &["src", "lib"];
const ENTRYPOINTS: &[&str] = &["package.json", "index.js", "index.mjs", "index.cjs"];

pub(super) const PROFILE: SurfaceLanguageProfile = SurfaceLanguageProfile {
    language: Language::JavaScript,
    noise_dirs: NOISE_DIRS,
    noise_file_suffixes: NOISE_FILE_SUFFIXES,
    drop_segments: DROP_SEGMENTS,
    drop_prefixes: DROP_PREFIXES,
    preferred_roots: PREFERRED_ROOTS,
    entrypoints: ENTRYPOINTS,
};
