use crate::types::Language;

use super::super::SurfaceLanguageProfile;

const NOISE_DIRS: &[&str] = &[
    "benches", "doc", "docs", "example", "examples", "target", "test", "tests",
];
const NOISE_FILE_SUFFIXES: &[&str] = &["_benchmark.rs", "_bench.rs", "_test.rs", "_tests.rs"];
const DROP_SEGMENTS: &[&str] = &["src"];
const PREFIX_SRC: &[&str] = &["src"];
const PREFIX_TESTS: &[&str] = &["tests"];
const PREFIX_EXAMPLES: &[&str] = &["examples"];
const PREFIX_BENCHES: &[&str] = &["benches"];
const DROP_PREFIXES: &[&[&str]] = &[PREFIX_SRC, PREFIX_TESTS, PREFIX_EXAMPLES, PREFIX_BENCHES];
const PREFERRED_ROOTS: &[&str] = &["src"];
const ENTRYPOINTS: &[&str] = &["Cargo.toml", "src/lib.rs", "lib.rs"];

pub(super) const PROFILE: SurfaceLanguageProfile = SurfaceLanguageProfile {
    language: Language::Rust,
    noise_dirs: NOISE_DIRS,
    noise_file_suffixes: NOISE_FILE_SUFFIXES,
    drop_segments: DROP_SEGMENTS,
    drop_prefixes: DROP_PREFIXES,
    preferred_roots: PREFERRED_ROOTS,
    entrypoints: ENTRYPOINTS,
};
