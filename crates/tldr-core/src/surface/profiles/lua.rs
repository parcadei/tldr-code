use crate::types::Language;

use super::super::SurfaceLanguageProfile;

const NOISE_DIRS: &[&str] = &[
    "bench",
    "benchmark",
    "benchmarks",
    "deps",
    "doc",
    "docs",
    "example",
    "examples",
    "spec",
    "specs",
    "test",
    "tests",
];
const NOISE_FILE_SUFFIXES: &[&str] = &["_bench.lua", "_benchmark.lua", "_spec.lua", "_test.lua"];
const DROP_SEGMENTS: &[&str] = &["lib", "lua"];
const PREFIX_LUA: &[&str] = &["lua"];
const PREFIX_LIB: &[&str] = &["lib"];
const DROP_PREFIXES: &[&[&str]] = &[PREFIX_LUA, PREFIX_LIB];
const PREFERRED_ROOTS: &[&str] = &["lua", "lib"];
const ENTRYPOINTS: &[&str] = &["rockspec", "init.lua", "lua", "lib"];

pub(super) const PROFILE: SurfaceLanguageProfile = SurfaceLanguageProfile {
    language: Language::Lua,
    noise_dirs: NOISE_DIRS,
    noise_file_suffixes: NOISE_FILE_SUFFIXES,
    drop_segments: DROP_SEGMENTS,
    drop_prefixes: DROP_PREFIXES,
    preferred_roots: PREFERRED_ROOTS,
    entrypoints: ENTRYPOINTS,
};
