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
    ".bench.ts",
    ".bench.tsx",
    ".benchmark.ts",
    ".benchmark.tsx",
    ".fixture.ts",
    ".fixture.tsx",
    ".spec.ts",
    ".spec.tsx",
    ".test.ts",
    ".test.tsx",
];
const DROP_SEGMENTS: &[&str] = &["dist", "esm", "index", "lib", "src"];
const PREFIX_SRC: &[&str] = &["src"];
const PREFIX_LIB: &[&str] = &["lib"];
const PREFIX_DIST: &[&str] = &["dist"];
const PREFIX_ESM: &[&str] = &["esm"];
const DROP_PREFIXES: &[&[&str]] = &[PREFIX_SRC, PREFIX_LIB, PREFIX_DIST, PREFIX_ESM];
const PREFERRED_ROOTS: &[&str] = &["src", "lib"];
const ENTRYPOINTS: &[&str] = &[
    "package.json",
    "index.ts",
    "index.tsx",
    "index.d.ts",
    "src/index.ts",
    "src/index.tsx",
    "types/index.d.ts",
];

pub(super) const PROFILE: SurfaceLanguageProfile = SurfaceLanguageProfile {
    language: Language::TypeScript,
    noise_dirs: NOISE_DIRS,
    noise_file_suffixes: NOISE_FILE_SUFFIXES,
    drop_segments: DROP_SEGMENTS,
    drop_prefixes: DROP_PREFIXES,
    preferred_roots: PREFERRED_ROOTS,
    entrypoints: ENTRYPOINTS,
};
