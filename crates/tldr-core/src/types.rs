//! Common types for TLDR operations
//!
//! This module defines all shared types used across the TLDR codebase.
//! All types derive Serialize/Deserialize with consistent field ordering
//! to address M5 (JSON Serialization Consistency).
//!
//! ## Submodules
//!
//! - `inheritance` - Types for class hierarchy extraction (Phase 7-9, A9)
//! - `patterns` - Types for design pattern mining (Phase 4-6, A10)
//! - `arch_rules` - Types for architecture rules and violations (Phase 3, A11)

// =============================================================================
// Submodules for Architecture Commands (Phase 1: Types Foundation)
// =============================================================================

pub mod arch_rules;
pub mod inheritance;
pub mod patterns;

// Re-export submodule types for convenience
pub use arch_rules::*;
pub use inheritance::*;
pub use patterns::*;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

// =============================================================================
// Language Support
// =============================================================================

/// Supported programming languages (17 variants as per spec Section 1.2)
///
/// Priority levels:
/// - P0: Python, TypeScript, JavaScript, Go (full support)
/// - P1: Rust, Java (full support)
/// - P2: C, C++, Ruby, Kotlin, Swift, C#, Scala, PHP, Lua, Luau, Elixir (basic support)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// Python (.py)
    Python,
    /// TypeScript (.ts, .tsx)
    TypeScript,
    /// JavaScript (.js, .jsx, .mjs, .cjs)
    JavaScript,
    /// Go (.go)
    Go,
    /// Rust (.rs)
    Rust,
    /// Java (.java)
    Java,
    /// C (.c, .h)
    C,
    /// C++ (.cpp, .cc, .cxx, .hpp)
    Cpp,
    /// Ruby (.rb)
    Ruby,
    /// Kotlin (.kt, .kts)
    Kotlin,
    /// Swift (.swift)
    Swift,
    /// C# (.cs)
    CSharp,
    /// Scala (.scala)
    Scala,
    /// PHP (.php)
    Php,
    /// Lua (.lua)
    Lua,
    /// Luau (.luau)
    Luau,
    /// Elixir (.ex, .exs)
    Elixir,
    /// OCaml (.ml, .mli)
    Ocaml,
}

impl Language {
    /// Get file extensions for this language
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            Language::Python => &[".py"],
            Language::TypeScript => &[".ts", ".tsx"],
            Language::JavaScript => &[".js", ".jsx", ".mjs", ".cjs"],
            Language::Go => &[".go"],
            Language::Rust => &[".rs"],
            Language::Java => &[".java"],
            Language::C => &[".c", ".h"],
            Language::Cpp => &[".cpp", ".cc", ".cxx", ".hpp"],
            Language::Ruby => &[".rb"],
            Language::Kotlin => &[".kt", ".kts"],
            Language::Swift => &[".swift"],
            Language::CSharp => &[".cs"],
            Language::Scala => &[".scala"],
            Language::Php => &[".php"],
            Language::Lua => &[".lua"],
            Language::Luau => &[".luau"],
            Language::Elixir => &[".ex", ".exs"],
            Language::Ocaml => &[".ml", ".mli"],
        }
    }

    /// Detect language from file extension
    pub fn from_extension(ext: &str) -> Option<Self> {
        // Normalize extension to lowercase with leading dot
        let ext = if ext.starts_with('.') {
            ext.to_lowercase()
        } else {
            format!(".{}", ext.to_lowercase())
        };

        match ext.as_str() {
            ".py" => Some(Language::Python),
            ".ts" | ".tsx" => Some(Language::TypeScript),
            ".js" | ".jsx" | ".mjs" | ".cjs" => Some(Language::JavaScript),
            ".go" => Some(Language::Go),
            ".rs" => Some(Language::Rust),
            ".java" => Some(Language::Java),
            ".c" | ".h" => Some(Language::C),
            ".cpp" | ".cc" | ".cxx" | ".hpp" => Some(Language::Cpp),
            ".rb" => Some(Language::Ruby),
            ".kt" | ".kts" => Some(Language::Kotlin),
            ".swift" => Some(Language::Swift),
            ".cs" => Some(Language::CSharp),
            ".scala" => Some(Language::Scala),
            ".php" => Some(Language::Php),
            ".lua" => Some(Language::Lua),
            ".luau" => Some(Language::Luau),
            ".ex" | ".exs" => Some(Language::Elixir),
            ".ml" | ".mli" => Some(Language::Ocaml),
            _ => None,
        }
    }

    /// Detect language from file path
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| Self::from_extension(&format!(".{}", ext)))
    }

    /// Detect dominant language from files in a directory.
    ///
    /// # Detection strategy (VAL-002)
    ///
    /// This is a two-stage detector designed to survive pnpm / npm / yarn
    /// monorepos whose `node_modules/.pnpm/**` trees ship thousands of
    /// `.py` files from `node-gyp` (which would otherwise win a naive
    /// extension vote even on a clearly TypeScript project).
    ///
    /// 1. **Manifest priority (preferred).** Scan the root, each immediate
    ///    subdirectory, and each grandchild (depth ≤ 2 — covers
    ///    `apps/*/` and `packages/*/` monorepo layouts) for a build
    ///    manifest. Among all manifests found, the one with the highest
    ///    precedence wins; ties at the same precedence are broken by
    ///    shallowest path.
    ///
    ///    | Precedence | Manifest(s)                                      | Language                          |
    ///    |-----------:|--------------------------------------------------|-----------------------------------|
    ///    |          1 | `tsconfig.json`                                  | TypeScript                        |
    ///    |          2 | `package.json`                                   | TypeScript (with TS dep) or JS    |
    ///    |          3 | `Cargo.toml`                                     | Rust                              |
    ///    |          4 | `go.mod`                                         | Go                                |
    ///    |        5–7 | `pyproject.toml`, `setup.py`, `requirements.txt` | Python                            |
    ///    |          8 | `pom.xml`                                        | Java                              |
    ///    |       9–10 | `build.gradle.kts`, `build.gradle`               | Kotlin or Java (tie-break below)  |
    ///    |      11–14 | `CMakeLists.txt`, `meson.build`, `configure.ac`/`configure.in`, `Makefile.am`/`Makefile.in` | C or C++ (tie-break below) |
    ///    |      15–17 | `*.csproj`, `*.sln`, `global.json` (with `sdk`)  | C#                                |
    ///    |      18–19 | `build.sbt`, `project/build.properties`          | Scala                             |
    ///    |      20–21 | `dune-project`, `*.opam`                         | OCaml                             |
    ///    |         22 | `Gemfile`                                        | Ruby                              |
    ///    |         23 | `composer.json`                                  | PHP                               |
    ///    |         24 | `mix.exs`                                        | Elixir                            |
    ///    |         25 | `Package.swift`                                  | Swift                             |
    ///    |      26–27 | `*.rockspec`, `.luarc.json`                      | Lua                               |
    ///    |      28–29 | `default.project.json` (Rojo), `.luaurc`         | Luau                              |
    ///
    ///    Gradle tie-break: when `build.gradle.kts` is the winning
    ///    manifest, count `.kt` vs `.java` files across the walk; pick
    ///    Kotlin when `.kt` > `.java`, else Java.
    ///
    ///    C/C++ tie-break: when one of the shared build-system manifests
    ///    (CMake, Meson, Autotools) wins, count `.cpp`/`.cc`/`.cxx`/`.hpp`/
    ///    `.hh`/`.hxx` vs `.c` files across the walk (`.h` is NOT counted —
    ///    it's ambiguous between C and C++). Pick C++ when the cpp-family
    ///    strictly exceeds the c-family; otherwise default to C.
    ///
    ///    *Why precedence beats depth.* In a pnpm monorepo the root
    ///    `package.json` usually holds tooling (turbo, prettier, eslint)
    ///    with no `typescript` dep, while the real language lives in
    ///    `packages/ui/tsconfig.json` and `apps/web/tsconfig.json`. A
    ///    depth-first rule would return JavaScript (wrong). Letting
    ///    `tsconfig.json` at depth 2 beat a plain `package.json` at
    ///    depth 0 returns TypeScript (correct).
    ///
    /// 2. **Extension-majority fallback.** If no manifest matched, walk
    ///    the directory via [`crate::walker::walk_project`] (skipping
    ///    `node_modules`/`target`/hidden/.gitignored paths, not following
    ///    symlinks), count file extensions, and return the majority
    ///    language. Returns `None` when the directory has no recognised
    ///    source files.
    pub fn from_directory(path: &std::path::Path) -> Option<Self> {
        // --- Stage 1: manifest priority ------------------------------------
        if let Some(lang) = detect_from_manifests(path) {
            return Some(lang);
        }

        // --- Stage 2: extension-majority fallback --------------------------
        use std::collections::HashMap;
        let mut counts: HashMap<Language, usize> = HashMap::new();
        for entry in crate::walker::walk_project(path) {
            let p = entry.path();
            if p.is_file() {
                if let Some(lang) = Self::from_path(p) {
                    *counts.entry(lang).or_insert(0) += 1;
                }
            }
        }

        counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(lang, _)| lang)
    }

    /// Get the language name as it appears in JSON output
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Go => "go",
            Language::Rust => "rust",
            Language::Java => "java",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Ruby => "ruby",
            Language::Kotlin => "kotlin",
            Language::Swift => "swift",
            Language::CSharp => "csharp",
            Language::Scala => "scala",
            Language::Php => "php",
            Language::Lua => "lua",
            Language::Luau => "luau",
            Language::Elixir => "elixir",
            Language::Ocaml => "ocaml",
        }
    }

    /// Check if this is a P0 (highest priority) language
    pub fn is_p0(&self) -> bool {
        matches!(
            self,
            Language::Python | Language::TypeScript | Language::JavaScript | Language::Go
        )
    }

    /// Check if this is a P1 (high priority) language
    pub fn is_p1(&self) -> bool {
        matches!(self, Language::Rust | Language::Java)
    }

    /// Get all supported languages
    pub fn all() -> &'static [Language] {
        &[
            Language::Python,
            Language::TypeScript,
            Language::JavaScript,
            Language::Go,
            Language::Rust,
            Language::Java,
            Language::C,
            Language::Cpp,
            Language::Ruby,
            Language::Kotlin,
            Language::Swift,
            Language::CSharp,
            Language::Scala,
            Language::Php,
            Language::Lua,
            Language::Luau,
            Language::Elixir,
            Language::Ocaml,
        ]
    }
}

// =============================================================================
// Manifest-based language detection (VAL-002)
//
// `Language::from_directory` uses these helpers as its first stage before
// falling back to extension majority. The goal is to beat false positives on
// pnpm/npm monorepos where `node_modules/.pnpm/**` ships thousands of `.py`
// files from node-gyp and wins a naive extension vote on TypeScript projects.
// =============================================================================

/// Manifest-file precedence when multiple candidates exist at the same depth.
///
/// Ordered from highest to lowest precedence. Earlier entries win ties in
/// `detect_from_manifests`; see `Language::from_directory` docs for the full
/// rationale.
///
/// VAL-008 expanded this list from 14 to 29 entries, adding manifest support
/// for the 7 previously extension-only languages (C, Cpp, CSharp, Scala,
/// OCaml, Lua, Luau). C and C++ share the same manifest families (CMake,
/// Meson, Autotools) and are disambiguated via a source-file-count tie-break
/// in `language_from_manifest_set`.
const MANIFEST_PRECEDENCE: &[ManifestKind] = &[
    // --- TS/JS (highest: most specific signal for the largest lang family)
    ManifestKind::TsConfig,
    ManifestKind::PackageJson,
    // --- Rust, Go
    ManifestKind::CargoToml,
    ManifestKind::GoMod,
    // --- Python
    ManifestKind::PyProject,
    ManifestKind::SetupPy,
    ManifestKind::RequirementsTxt,
    // --- JVM (Java/Kotlin)
    ManifestKind::PomXml,
    ManifestKind::BuildGradleKts,
    ManifestKind::BuildGradle,
    // --- C / C++ build systems (shared manifests, tie-break by file count).
    // Placed high because CMake/Meson/Autotools are unambiguous C-family signals.
    ManifestKind::CmakeLists,
    ManifestKind::MesonBuild,
    ManifestKind::ConfigureAc,
    ManifestKind::MakefileAm,
    // --- CSharp (high-signal, specific)
    ManifestKind::CsProj,
    ManifestKind::SlnFile,
    ManifestKind::GlobalJson,
    // --- Scala
    ManifestKind::BuildSbt,
    ManifestKind::ScalaBuildProperties,
    // --- OCaml
    ManifestKind::DuneProject,
    ManifestKind::OpamFile,
    // --- Ruby, PHP, Elixir, Swift (pre-VAL-008 order, preserved)
    ManifestKind::Gemfile,
    ManifestKind::ComposerJson,
    ManifestKind::MixExs,
    ManifestKind::PackageSwift,
    // --- Lua / Luau (weakest signals — lua projects often lack formal manifests)
    ManifestKind::Rockspec,
    ManifestKind::Luarc,
    ManifestKind::RojoProject,
    ManifestKind::LuauRc,
];

/// How a `ManifestKind` matches a directory entry.
///
/// Most manifests are identified by a fixed filename (`Cargo.toml`, `go.mod`);
/// a few are identified by extension (`*.csproj`, `*.opam`, `*.rockspec`); and
/// one — `ScalaBuildProperties` — is a nested fixed filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestMatcher {
    /// Exact filename match at the directory root (e.g. `Cargo.toml`).
    Exact(&'static str),
    /// File extension match, e.g. `"csproj"` matches `MyApp.csproj`.
    /// Matching is case-insensitive.
    Extension(&'static str),
    /// Nested fixed path under the directory (e.g. `project/build.properties`).
    Nested(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestKind {
    // Pre-VAL-008 (14 entries)
    TsConfig,
    PackageJson,
    CargoToml,
    GoMod,
    PyProject,
    SetupPy,
    RequirementsTxt,
    PomXml,
    BuildGradle,
    BuildGradleKts,
    Gemfile,
    ComposerJson,
    MixExs,
    PackageSwift,
    // VAL-008: C / C++ (shared; language chosen by extension tie-break)
    CmakeLists,
    MesonBuild,
    ConfigureAc, // matches both `configure.ac` and `configure.in`
    MakefileAm,  // matches both `Makefile.am` and `Makefile.in`
    // VAL-008: CSharp
    CsProj,     // *.csproj (extension match)
    SlnFile,    // *.sln (extension match)
    GlobalJson, // global.json — only counts when `"sdk"` key is present
    // VAL-008: Scala
    BuildSbt,
    ScalaBuildProperties, // nested: project/build.properties
    // VAL-008: OCaml
    DuneProject,
    OpamFile, // *.opam (extension match)
    // VAL-008: Lua
    Rockspec, // *.rockspec (extension match)
    Luarc,    // .luarc.json
    // VAL-008: Luau
    RojoProject, // default.project.json
    LuauRc,      // .luaurc
}

impl ManifestKind {
    /// The matcher used to locate this manifest in a directory.
    fn matcher(self) -> ManifestMatcher {
        match self {
            // Fixed-filename manifests (pre-VAL-008)
            ManifestKind::TsConfig => ManifestMatcher::Exact("tsconfig.json"),
            ManifestKind::PackageJson => ManifestMatcher::Exact("package.json"),
            ManifestKind::CargoToml => ManifestMatcher::Exact("Cargo.toml"),
            ManifestKind::GoMod => ManifestMatcher::Exact("go.mod"),
            ManifestKind::PyProject => ManifestMatcher::Exact("pyproject.toml"),
            ManifestKind::SetupPy => ManifestMatcher::Exact("setup.py"),
            ManifestKind::RequirementsTxt => ManifestMatcher::Exact("requirements.txt"),
            ManifestKind::PomXml => ManifestMatcher::Exact("pom.xml"),
            ManifestKind::BuildGradle => ManifestMatcher::Exact("build.gradle"),
            ManifestKind::BuildGradleKts => ManifestMatcher::Exact("build.gradle.kts"),
            ManifestKind::Gemfile => ManifestMatcher::Exact("Gemfile"),
            ManifestKind::ComposerJson => ManifestMatcher::Exact("composer.json"),
            ManifestKind::MixExs => ManifestMatcher::Exact("mix.exs"),
            ManifestKind::PackageSwift => ManifestMatcher::Exact("Package.swift"),
            // VAL-008: C / C++ build systems
            ManifestKind::CmakeLists => ManifestMatcher::Exact("CMakeLists.txt"),
            ManifestKind::MesonBuild => ManifestMatcher::Exact("meson.build"),
            // configure.ac / configure.in: handled as a special case in
            // `matches_in` because it's a two-filename disjunction, not a
            // true extension match.
            ManifestKind::ConfigureAc => ManifestMatcher::Exact("configure.ac"),
            ManifestKind::MakefileAm => ManifestMatcher::Exact("Makefile.am"),
            // VAL-008: CSharp
            ManifestKind::CsProj => ManifestMatcher::Extension("csproj"),
            ManifestKind::SlnFile => ManifestMatcher::Extension("sln"),
            ManifestKind::GlobalJson => ManifestMatcher::Exact("global.json"),
            // VAL-008: Scala
            ManifestKind::BuildSbt => ManifestMatcher::Exact("build.sbt"),
            ManifestKind::ScalaBuildProperties => {
                ManifestMatcher::Nested("project/build.properties")
            }
            // VAL-008: OCaml
            ManifestKind::DuneProject => ManifestMatcher::Exact("dune-project"),
            ManifestKind::OpamFile => ManifestMatcher::Extension("opam"),
            // VAL-008: Lua
            ManifestKind::Rockspec => ManifestMatcher::Extension("rockspec"),
            ManifestKind::Luarc => ManifestMatcher::Exact(".luarc.json"),
            // VAL-008: Luau
            ManifestKind::RojoProject => ManifestMatcher::Exact("default.project.json"),
            ManifestKind::LuauRc => ManifestMatcher::Exact(".luaurc"),
        }
    }

    /// True when a directory contains a file matching this manifest kind.
    ///
    /// Handles the three matcher flavours:
    /// - `Exact`: single `path.join(name).is_file()` check.
    /// - `Extension`: scan `read_dir` entries for any file with that extension.
    /// - `Nested`: single `path.join(rel).is_file()` check.
    ///
    /// Also handles the special-case disjunctions:
    /// - `ConfigureAc` matches either `configure.ac` or `configure.in`.
    /// - `MakefileAm` matches either `Makefile.am` or `Makefile.in`.
    /// - `GlobalJson` additionally requires `"sdk"` to appear as a JSON key,
    ///   to avoid false-positives on unrelated `global.json` files shipped
    ///   by tools like `expo-cli` or `firebase-tools`.
    fn matches_in(self, dir: &std::path::Path) -> bool {
        match self {
            ManifestKind::ConfigureAc => {
                dir.join("configure.ac").is_file() || dir.join("configure.in").is_file()
            }
            ManifestKind::MakefileAm => {
                dir.join("Makefile.am").is_file() || dir.join("Makefile.in").is_file()
            }
            ManifestKind::GlobalJson => {
                let p = dir.join("global.json");
                if !p.is_file() {
                    return false;
                }
                // Require an "sdk" key: that's the unambiguous .NET marker.
                // Readers that fail for any reason fall through to false so
                // we don't mis-tag unrelated global.json files as CSharp.
                match std::fs::read_to_string(&p) {
                    Ok(contents) => global_json_has_sdk_key(&contents),
                    Err(_) => false,
                }
            }
            _ => match self.matcher() {
                ManifestMatcher::Exact(name) => dir.join(name).is_file(),
                ManifestMatcher::Nested(rel) => dir.join(rel).is_file(),
                ManifestMatcher::Extension(ext) => dir_has_file_with_extension(dir, ext),
            },
        }
    }
}

/// Scan `dir` (non-recursive) for any file whose extension matches `ext`
/// (case-insensitive, without a leading dot). Used by extension-matching
/// manifests such as `*.csproj`, `*.opam`, `*.rockspec`.
fn dir_has_file_with_extension(dir: &std::path::Path, ext: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let target = ext.to_ascii_lowercase();
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        if let Some(e) = p.extension().and_then(|e| e.to_str()) {
            if e.to_ascii_lowercase() == target {
                return true;
            }
        }
    }
    false
}

/// Check whether a `global.json` contents string has an `"sdk"` key. The
/// test is conservative: the file must parse as a JSON object AND that
/// object must contain `sdk` as a top-level key. This avoids false positives
/// on unrelated `global.json` files shipped by other tooling.
fn global_json_has_sdk_key(contents: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(contents) {
        Ok(serde_json::Value::Object(map)) => map.contains_key("sdk"),
        _ => false,
    }
}

/// Decide C vs. C++ for a project whose winning manifest is one of the
/// shared C/C++ families (CMake, Meson, Autotools).
///
/// Strategy: count source files in the project walk by family.
/// - C++ family: `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hh`, `.hxx`.
/// - C family:   `.c` (NOT `.h` — ambiguous with C++).
///
/// If the C++ count strictly exceeds the C count, return `Cpp`; otherwise
/// default to `C` (the older, simpler language wins on ties or empty counts).
fn c_vs_cpp_tie_break(root: &std::path::Path) -> Language {
    let mut c_family = 0usize;
    let mut cpp_family = 0usize;
    for entry in crate::walker::walk_project(root) {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        match p.extension().and_then(|e| e.to_str()) {
            Some("cpp") | Some("cc") | Some("cxx") | Some("hpp") | Some("hh") | Some("hxx") => {
                cpp_family += 1
            }
            Some("c") => c_family += 1,
            _ => {}
        }
    }
    if cpp_family > c_family {
        Language::Cpp
    } else {
        Language::C
    }
}

/// Collect the immediate-child directories of `parent`, skipping hidden
/// directories and the well-known vendor list so monorepo sub-manifests
/// buried in `node_modules` / `target` can't mask the real project.
fn collect_child_dirs(parent: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(parent) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') || crate::walker::DEFAULT_EXCLUDE_DIRS.contains(&name) {
                continue;
            }
        }
        out.push(p);
    }
    out
}

/// Look for a project manifest at `root` and at immediate subdirectories
/// (depth <= 2 to cover pnpm/Yarn/Turbo monorepos that keep manifests in
/// `packages/*/` and `apps/*/`).
///
/// Returns `None` when no manifest is found; callers fall back to
/// extension-majority detection. Precedence works as follows (VAL-002):
///
/// 1. Collect every manifest at every scanned depth.
/// 2. Pick the one with the highest slot in [`MANIFEST_PRECEDENCE`]
///    (TsConfig > PackageJson > CargoToml > ...). This means a
///    `tsconfig.json` nested in `packages/ui/` beats a bare
///    `package.json` at the root — the correct outcome for monorepos
///    where the root package.json holds only tooling (prettier, turbo,
///    eslint) and the language lives in subpackages.
/// 3. If multiple manifests share the top precedence, pick the shallowest
///    path. (Purely cosmetic — the same manifest at any depth resolves
///    to the same language.)
fn detect_from_manifests(root: &std::path::Path) -> Option<Language> {
    // Collect candidate directories at depth 0 (root), depth 1 (immediate
    // subdirs), and depth 2 (grandchildren — needed for `packages/*/`).
    let mut dirs: Vec<(usize, std::path::PathBuf)> = Vec::new();
    dirs.push((0, root.to_path_buf()));

    let depth1 = collect_child_dirs(root);
    for d1 in &depth1 {
        dirs.push((1, d1.clone()));
    }
    for d1 in &depth1 {
        for d2 in collect_child_dirs(d1) {
            dirs.push((2, d2));
        }
    }

    // Collect every (precedence_index, depth, dir, manifest) tuple, then
    // pick the entry with the smallest precedence_index, breaking ties by
    // shallowest depth.
    let mut best: Option<(usize, usize, std::path::PathBuf, ManifestKind)> = None;
    for (depth, dir) in &dirs {
        for (idx, m) in MANIFEST_PRECEDENCE.iter().copied().enumerate() {
            if m.matches_in(dir) {
                let candidate = (idx, *depth, dir.clone(), m);
                best = match best {
                    None => Some(candidate),
                    Some(ref existing) => {
                        // Pick the lower precedence index, then shallower depth.
                        if candidate.0 < existing.0
                            || (candidate.0 == existing.0 && candidate.1 < existing.1)
                        {
                            Some(candidate)
                        } else {
                            Some(existing.clone())
                        }
                    }
                };
            }
        }
    }

    best.and_then(|(_, _, dir, m)| language_from_manifest_set(&dir, &[m], root))
}

/// Convert a sorted-by-precedence set of manifests (all at the same depth)
/// into a [`Language`], applying the per-manifest heuristics.
///
/// The `project_root` is the original path passed to `from_directory` and is
/// used to count `.kt` vs `.java` files when resolving Gradle ambiguity.
///
/// `present` is assumed non-empty; the first manifest wins since it has
/// highest precedence per [`MANIFEST_PRECEDENCE`].
fn language_from_manifest_set(
    dir: &std::path::Path,
    present: &[ManifestKind],
    project_root: &std::path::Path,
) -> Option<Language> {
    let m = *present.first()?;
    let lang = match m {
        ManifestKind::TsConfig => Language::TypeScript,
        ManifestKind::PackageJson => {
            // TypeScript when a typescript dep is declared, else JavaScript.
            // If we can't read the file, assume JavaScript.
            let p = dir.join("package.json");
            match std::fs::read_to_string(&p) {
                Ok(contents) if package_json_has_typescript_dep(&contents) => Language::TypeScript,
                _ => Language::JavaScript,
            }
        }
        ManifestKind::CargoToml => Language::Rust,
        ManifestKind::GoMod => Language::Go,
        ManifestKind::PyProject | ManifestKind::SetupPy | ManifestKind::RequirementsTxt => {
            Language::Python
        }
        ManifestKind::PomXml => Language::Java,
        ManifestKind::BuildGradleKts => {
            // Kotlin DSL Gradle file; could be either Kotlin or Java.
            // Tie-break by counting .kt vs .java across the project.
            gradle_kotlin_vs_java(project_root)
        }
        ManifestKind::BuildGradle => Language::Java,
        ManifestKind::Gemfile => Language::Ruby,
        ManifestKind::ComposerJson => Language::Php,
        ManifestKind::MixExs => Language::Elixir,
        ManifestKind::PackageSwift => Language::Swift,
        // VAL-008: C / C++ shared build-system manifests. Dispatch via
        // file-count tie-break (`.cpp`/`.cc`/`.cxx`/`.hpp`/`.hh`/`.hxx` vs
        // `.c`). On ties or empty counts we fall back to C.
        ManifestKind::CmakeLists
        | ManifestKind::MesonBuild
        | ManifestKind::ConfigureAc
        | ManifestKind::MakefileAm => c_vs_cpp_tie_break(project_root),
        // VAL-008: CSharp
        ManifestKind::CsProj | ManifestKind::SlnFile | ManifestKind::GlobalJson => Language::CSharp,
        // VAL-008: Scala
        ManifestKind::BuildSbt | ManifestKind::ScalaBuildProperties => Language::Scala,
        // VAL-008: OCaml
        ManifestKind::DuneProject | ManifestKind::OpamFile => Language::Ocaml,
        // VAL-008: Lua
        ManifestKind::Rockspec | ManifestKind::Luarc => Language::Lua,
        // VAL-008: Luau
        ManifestKind::RojoProject | ManifestKind::LuauRc => Language::Luau,
    };
    Some(lang)
}

/// Lightweight check for "typescript" as a dep / devDep / peerDep in a
/// `package.json`. We avoid pulling in a JSON parser for this — a substring
/// check inside the dependency-section braces is enough to separate the
/// common TS project case from a pure-JS project.
fn package_json_has_typescript_dep(contents: &str) -> bool {
    // Fast reject: the word "typescript" must appear somewhere.
    if !contents.contains("typescript") {
        return false;
    }
    // Simple heuristic: look for `"typescript"` as a JSON key (followed by
    // a colon with optional whitespace). Matches:
    //   "typescript": "5.0.0"
    //   "typescript" : "^5"
    // Won't match a dep whose *value* contains the word (e.g. a package
    // called "my-typescript-helper" would only trip the fast-reject, not
    // this check).
    let mut rest = contents;
    while let Some(idx) = rest.find("\"typescript\"") {
        let after = &rest[idx + "\"typescript\"".len()..];
        let trimmed = after.trim_start();
        if trimmed.starts_with(':') {
            return true;
        }
        rest = after;
    }
    false
}

/// Resolve Gradle ambiguity: return Kotlin when `.kt` file count across the
/// project walk exceeds `.java`, otherwise Java.
fn gradle_kotlin_vs_java(root: &std::path::Path) -> Language {
    let mut kt = 0usize;
    let mut java = 0usize;
    for entry in crate::walker::walk_project(root) {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        match p.extension().and_then(|e| e.to_str()) {
            Some("kt") | Some("kts") => kt += 1,
            Some("java") => java += 1,
            _ => {}
        }
    }
    if kt > java {
        Language::Kotlin
    } else {
        Language::Java
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for Language {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "python" | "py" => Ok(Language::Python),
            "typescript" | "ts" => Ok(Language::TypeScript),
            "javascript" | "js" => Ok(Language::JavaScript),
            "go" | "golang" => Ok(Language::Go),
            "rust" | "rs" => Ok(Language::Rust),
            "java" => Ok(Language::Java),
            "c" => Ok(Language::C),
            "cpp" | "c++" | "cxx" => Ok(Language::Cpp),
            "ruby" | "rb" => Ok(Language::Ruby),
            "kotlin" | "kt" => Ok(Language::Kotlin),
            "swift" => Ok(Language::Swift),
            "csharp" | "c#" | "cs" => Ok(Language::CSharp),
            "scala" => Ok(Language::Scala),
            "php" => Ok(Language::Php),
            "lua" => Ok(Language::Lua),
            "luau" => Ok(Language::Luau),
            "elixir" | "ex" => Ok(Language::Elixir),
            "ocaml" | "ml" => Ok(Language::Ocaml),
            _ => Err(format!("Unknown language: {}", s)),
        }
    }
}

// =============================================================================
// File System Types
// =============================================================================

/// File tree node type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    /// Directory node
    Dir,
    /// File node
    File,
}

/// File tree structure (spec Section 2.1.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTree {
    /// Display name of the file or directory
    pub name: String,
    /// Whether this node is a file or directory
    #[serde(rename = "type")]
    pub node_type: NodeType,
    /// Absolute path to the file (None for directory nodes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Child nodes (only populated for directory nodes)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<FileTree>,
}

impl FileTree {
    /// Create a new file node
    pub fn file(name: impl Into<String>, path: PathBuf) -> Self {
        Self {
            name: name.into(),
            node_type: NodeType::File,
            path: Some(path),
            children: Vec::new(),
        }
    }

    /// Create a new directory node
    pub fn dir(name: impl Into<String>, children: Vec<FileTree>) -> Self {
        Self {
            name: name.into(),
            node_type: NodeType::Dir,
            path: None,
            children,
        }
    }
}

/// File entry for flat file lists
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Path to the file
    pub path: PathBuf,
    /// Detected programming language, if any
    pub language: Option<Language>,
    /// File size in bytes
    pub size_bytes: u64,
}

/// Ignore specification (gitignore-style patterns)
#[derive(Debug, Clone, Default)]
pub struct IgnoreSpec {
    /// Glob patterns for files and directories to ignore
    pub patterns: Vec<String>,
}

impl IgnoreSpec {
    /// Create a new ignore spec from patterns
    pub fn new(patterns: Vec<String>) -> Self {
        Self { patterns }
    }

    /// Load from a file (like .tldrignore or .gitignore)
    pub fn from_file(_path: &std::path::Path) -> std::io::Result<Self> {
        // TODO: Implement in Phase 2
        Ok(Self::default())
    }

    /// Check if a path should be ignored
    pub fn is_ignored(&self, _path: &std::path::Path) -> bool {
        // TODO: Implement pattern matching in Phase 2
        false
    }
}

// =============================================================================
// AST Types (Layer 1)
// =============================================================================

/// Code structure for a project (spec Section 2.1.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeStructure {
    /// Root directory of the analyzed project
    pub root: PathBuf,
    /// Primary programming language of the project
    pub language: Language,
    /// Structural information for each source file
    pub files: Vec<FileStructure>,
}

/// Definition-level information with line ranges and signatures.
/// Extracted from tree-sitter AST, suitable for caching.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefinitionInfo {
    /// Symbol name
    pub name: String,
    /// Kind: "function", "method", "class", "struct"
    pub kind: String,
    /// Start line (1-indexed)
    pub line_start: u32,
    /// End line (1-indexed, inclusive)
    pub line_end: u32,
    /// Signature line (e.g., "pub fn foo(x: i32) -> bool")
    pub signature: String,
}

/// Structure of a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStructure {
    /// Path to the source file
    pub path: PathBuf,
    /// Names of top-level functions defined in this file
    pub functions: Vec<String>,
    /// Names of classes or structs defined in this file
    pub classes: Vec<String>,
    /// Names of methods (functions inside classes) in this file
    pub methods: Vec<String>,
    /// Import statements found in this file
    pub imports: Vec<ImportInfo>,
    /// Detailed definition information with line ranges and signatures
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub definitions: Vec<DefinitionInfo>,
}

/// Import statement information (spec Section 2.1.4)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportInfo {
    /// Module or package being imported
    pub module: String,
    /// Specific names imported from the module (e.g., `from X import a, b`)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    /// Whether this is a `from` import (e.g., `from module import name`)
    #[serde(default)]
    pub is_from: bool,
    /// Import alias (e.g., `import X as Y`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// Complete module information (spec Section 2.1.3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    /// Path to the source file for this module
    pub file_path: PathBuf,
    /// Programming language of the module
    pub language: Language,
    /// Module-level docstring, if present
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// Import statements in this module
    pub imports: Vec<ImportInfo>,
    /// Top-level functions defined in this module
    pub functions: Vec<FunctionInfo>,
    /// Classes or structs defined in this module
    pub classes: Vec<ClassInfo>,
    /// Module-level constants (Gap 3)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constants: Vec<FieldInfo>,
    /// Intra-file call graph showing function call relationships within this module
    pub call_graph: IntraFileCallGraph,
}

/// Function information with full details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    /// Name of the function
    pub name: String,
    /// Parameter names (and optional type annotations)
    pub params: Vec<String>,
    /// Return type annotation, if present
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    /// Docstring or doc comment for this function
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// Whether this function is a method (defined inside a class/struct)
    #[serde(default)]
    pub is_method: bool,
    /// Whether this function is declared as async
    #[serde(default)]
    pub is_async: bool,
    /// Decorator or annotation names applied to this function
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decorators: Vec<String>,
    /// Line number where this function is defined (1-indexed)
    pub line_number: u32,
}

/// Class information with full details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassInfo {
    /// Name of the class or struct
    pub name: String,
    /// Base classes or parent types this class extends
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bases: Vec<String>,
    /// Docstring or doc comment for this class
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// Methods defined in this class
    pub methods: Vec<FunctionInfo>,
    /// Fields/properties of the class (Gap 3)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldInfo>,
    /// Decorator or annotation names applied to this class
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decorators: Vec<String>,
    /// Line number where this class is defined (1-indexed)
    pub line_number: u32,
}

/// Field or constant information (Gap 3)
///
/// Represents:
/// - Class/struct fields (instance variables, properties)
/// - Module-level constants
/// - Static class variables
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldInfo {
    /// Field name
    pub name: String,
    /// Field type annotation (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_type: Option<String>,
    /// Default value (if present)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    /// Whether this is a static/class variable
    #[serde(default)]
    pub is_static: bool,
    /// Whether this is a constant (immutable, UPPER_CASE by convention)
    #[serde(default)]
    pub is_constant: bool,
    /// Visibility modifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    /// Line number where field is defined (1-indexed)
    pub line_number: u32,
}

/// Intra-file call graph
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntraFileCallGraph {
    /// Map from function name to the list of functions it calls
    pub calls: HashMap<String, Vec<String>>,
    /// Reverse map from function name to the list of functions that call it
    pub called_by: HashMap<String, Vec<String>>,
}

// =============================================================================
// Call Graph Types (Layer 2)
// =============================================================================

/// Helper for serde skip_serializing_if on u32 fields.
fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// Reference to a function in the codebase, used in call graphs and dead code analysis.
///
/// Equality and hashing are based only on `file` and `name`, so metadata
/// fields do not affect `HashSet`/`HashMap` lookups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionRef {
    /// Path to the file containing this function
    pub file: PathBuf,
    /// Name of the function
    pub name: String,
    /// Line number where the function starts (1-based, 0 = unknown)
    #[serde(default)]
    pub line: u32,
    /// Function signature (e.g. "def my_func(x, y) -> int")
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature: String,
    /// Reference count: how many times this identifier appears across the codebase.
    /// 1 = only the definition, 0 = unknown/not computed.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub ref_count: u32,
    /// Whether this function is public/exported (pub, export, uppercase Go, etc.)
    #[serde(default)]
    pub is_public: bool,
    /// Whether this function is a test function (in test file or test function)
    #[serde(default)]
    pub is_test: bool,
    /// Whether this function is inside a trait/interface/protocol/abstract class
    #[serde(default)]
    pub is_trait_method: bool,
    /// Whether this function has any decorator/annotation
    #[serde(default)]
    pub has_decorator: bool,
    /// Names of decorators/annotations on this function
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decorator_names: Vec<String>,
}

// Equality based on file + name only (metadata is for analysis, not identity)
impl PartialEq for FunctionRef {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file && self.name == other.name
    }
}

impl Eq for FunctionRef {}

// Hash based on file + name only (must match PartialEq)
impl std::hash::Hash for FunctionRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.file.hash(state);
        self.name.hash(state);
    }
}

impl FunctionRef {
    /// Create a new function reference with default (unenriched) metadata.
    ///
    /// All metadata fields default to false/empty, meaning the function
    /// is treated as private with no special attributes. This preserves
    /// backward compatibility with existing call sites.
    pub fn new(file: PathBuf, name: impl Into<String>) -> Self {
        Self {
            file,
            name: name.into(),
            line: 0,
            signature: String::new(),
            ref_count: 0,
            is_public: false,
            is_test: false,
            is_trait_method: false,
            has_decorator: false,
            decorator_names: Vec::new(),
        }
    }
}

impl std::fmt::Display for FunctionRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.file.display(), self.name)
    }
}

/// Workspace configuration for multi-root projects
#[derive(Debug, Clone, Default)]
pub struct WorkspaceConfig {
    /// Root directories of the workspace
    pub roots: Vec<PathBuf>,
}

/// Upper bound on workspace members to guard against pathological configs
/// (VAL-007). Matches the cost budget documented on `WorkspaceConfig::discover`.
const MAX_WORKSPACE_MEMBERS: usize = 256;

/// Directories we refuse to expand globs into during workspace discovery
/// (VAL-007). Mirrors [`crate::walker::DEFAULT_EXCLUDE_DIRS`] but is kept
/// duplicated here to avoid a circular module reference (`walker` depends
/// on nothing from `types`, and we want to keep it that way).
const WORKSPACE_EXPANSION_EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "__pycache__",
    "vendor",
    ".git",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
];

impl WorkspaceConfig {
    /// Discover workspace roots from common manifest files at or near `root`.
    ///
    /// Returns `Some(WorkspaceConfig { roots: [...] })` when a known workspace
    /// manifest is found and enumerates at least one member; `None` otherwise
    /// (so callers can preserve existing single-root behavior).
    ///
    /// Probed markers, in order:
    /// - `pnpm-workspace.yaml` at `root` (parses `packages:` glob list)
    /// - `package.json` at `root` with `"workspaces": [...]` (npm/yarn/pnpm)
    /// - `Cargo.toml` at `root` with `[workspace] members = [...]`
    /// - `go.work` at `root` with `use <path>` directives
    ///
    /// All returned roots are absolute paths (canonicalized when possible,
    /// falling back to `root.join(member)` when canonicalization fails).
    /// The returned list always contains `root` itself as the first entry,
    /// followed by each discovered member directory.
    ///
    /// To prevent pathological configurations, the returned list is capped
    /// at [`MAX_WORKSPACE_MEMBERS`] entries (including the root).
    pub fn discover(root: &Path) -> Option<Self> {
        // Deliberately do NOT canonicalize the root here. Downstream code
        // (e.g. `callgraph::scanner::resolve_scan_roots`) verifies that
        // every workspace root `starts_with(root)` using the path shape
        // the caller provided — canonicalizing to `/private/var/...` on
        // macOS when the caller passed `/var/...` would break that check.
        // The paths we return are always `root.join(member)` in the same
        // shape as the caller's root.
        let probe_root = root;

        // Probe markers in priority order.
        let members = probe_pnpm_workspace(probe_root)
            .or_else(|| probe_package_json_workspaces(probe_root))
            .or_else(|| probe_cargo_workspace(probe_root))
            .or_else(|| probe_go_work(probe_root))?;

        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut roots = Vec::with_capacity(members.len() + 1);

        // Always include the root itself so siblings can be scanned together.
        let root_key = root.to_path_buf();
        seen.insert(root_key.clone());
        roots.push(root_key);

        let cap = MAX_WORKSPACE_MEMBERS;
        let mut truncated = false;
        for member in members {
            if roots.len() >= cap {
                truncated = true;
                break;
            }
            if !member.exists() || !member.is_dir() {
                continue;
            }
            // Keep paths in the same shape as the caller's root so
            // downstream starts_with() checks pass (no canonicalization).
            if seen.insert(member.clone()) {
                roots.push(member);
            }
        }

        if truncated {
            eprintln!(
                "[tldr] WorkspaceConfig::discover: workspace member count exceeded {} — truncating; some roots not scanned.",
                cap
            );
        }

        // If only the root itself made it into the list (no real members
        // found), tell the caller there's no workspace.
        if roots.len() <= 1 {
            return None;
        }

        Some(Self { roots })
    }
}

// =============================================================================
// Workspace discovery probes (VAL-007)
// =============================================================================

/// Probe for a pnpm workspace at `root`. Returns discovered member directories
/// (NOT including the root itself) on success.
fn probe_pnpm_workspace(root: &Path) -> Option<Vec<PathBuf>> {
    let path = root.join("pnpm-workspace.yaml");
    let content = std::fs::read_to_string(&path).ok()?;

    // Parse with serde_yaml first; fall back to a minimal regex extractor
    // if the YAML is unparseable (pnpm sometimes accepts slightly sloppy
    // YAML that real-world repos have on disk).
    let packages: Vec<String> = serde_yaml::from_str::<serde_yaml::Value>(&content)
        .ok()
        .and_then(|v| {
            v.get("packages").and_then(|p| p.as_sequence()).map(|seq| {
                seq.iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_else(|| fallback_extract_yaml_list(&content, "packages"));

    if packages.is_empty() {
        return None;
    }

    Some(expand_workspace_patterns(root, &packages))
}

/// Probe for an npm/yarn `package.json` with a `workspaces` array.
fn probe_package_json_workspaces(root: &Path) -> Option<Vec<PathBuf>> {
    let path = root.join("package.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let ws = json.get("workspaces")?;

    // `workspaces` may be an array of strings OR an object with `packages`.
    let patterns: Vec<String> = if let Some(arr) = ws.as_array() {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    } else if let Some(obj) = ws.as_object() {
        obj.get("packages")
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        return None;
    };

    if patterns.is_empty() {
        return None;
    }

    Some(expand_workspace_patterns(root, &patterns))
}

/// Probe for a Cargo workspace at `root`. Parses the `[workspace]` section
/// manually to avoid pulling in the `toml` crate (the codebase already
/// reads Cargo.toml this way — see `detect_rust_crate_name` in
/// `callgraph/module_index.rs`).
fn probe_cargo_workspace(root: &Path) -> Option<Vec<PathBuf>> {
    let path = root.join("Cargo.toml");
    let content = std::fs::read_to_string(&path).ok()?;

    let mut in_workspace = false;
    let mut members_block: Option<String> = None;
    let mut buffer = String::new();
    let mut collecting = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if collecting {
                // A new section started before the array closed — bail.
                break;
            }
            in_workspace = trimmed == "[workspace]";
            continue;
        }
        if !in_workspace {
            continue;
        }

        if !collecting {
            // Look for `members = [...]` (possibly on a single line or multi-line).
            if let Some(rest) = trimmed.strip_prefix("members") {
                let after_eq = rest.trim_start().strip_prefix('=')?.trim_start();
                if let Some(after_open) = after_eq.strip_prefix('[') {
                    // Check if the array closes on the same line.
                    if let Some(end) = after_open.find(']') {
                        members_block = Some(after_open[..end].to_string());
                        break;
                    } else {
                        buffer.push_str(after_open);
                        buffer.push('\n');
                        collecting = true;
                    }
                }
            }
        } else if let Some(end) = trimmed.find(']') {
            buffer.push_str(&trimmed[..end]);
            members_block = Some(std::mem::take(&mut buffer));
            break;
        } else {
            buffer.push_str(trimmed);
            buffer.push('\n');
        }
    }

    let block = members_block?;
    let patterns: Vec<String> = block
        .split(',')
        .map(|p| {
            p.trim()
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string()
        })
        .filter(|p| !p.is_empty() && !p.starts_with('#'))
        .collect();

    if patterns.is_empty() {
        return None;
    }

    Some(expand_workspace_patterns(root, &patterns))
}

/// Probe for a Go workspace at `root` (`go.work`).
fn probe_go_work(root: &Path) -> Option<Vec<PathBuf>> {
    let path = root.join("go.work");
    let content = std::fs::read_to_string(&path).ok()?;

    let mut patterns: Vec<String> = Vec::new();
    let mut in_use_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        // Multi-line form: `use (\n\t./a\n\t./b\n)`
        if in_use_block {
            if trimmed == ")" {
                in_use_block = false;
                continue;
            }
            let p = trimmed.trim_matches('"').trim();
            if !p.is_empty() {
                patterns.push(p.to_string());
            }
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("use") {
            let rest = rest.trim_start();
            if rest == "(" || rest.is_empty() {
                in_use_block = rest == "(";
                continue;
            }
            // Single-line form: `use ./foo`
            let p = rest.trim_matches('"').trim();
            if !p.is_empty() {
                patterns.push(p.to_string());
            }
        }
    }

    if patterns.is_empty() {
        return None;
    }

    Some(expand_workspace_patterns(root, &patterns))
}

/// Expand a list of workspace patterns (possibly containing `*` globs or
/// `**` recursive globs) relative to `root` into a concrete list of
/// directory paths. Vendored / build directories are skipped.
fn expand_workspace_patterns(root: &Path, patterns: &[String]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();

    for pat in patterns {
        let cleaned = pat
            .trim()
            .trim_start_matches("./")
            .trim_end_matches('/')
            .to_string();
        if cleaned.is_empty() {
            continue;
        }

        if contains_glob_char(&cleaned) {
            let full = root.join(&cleaned);
            let full_str = full.to_string_lossy();
            if let Ok(paths) = glob::glob(&full_str) {
                for entry in paths.flatten() {
                    if entry.is_dir() && !path_contains_excluded_dir(root, &entry) {
                        out.push(entry);
                    }
                }
            }
        } else {
            let full = root.join(&cleaned);
            if full.is_dir() && !path_contains_excluded_dir(root, &full) {
                out.push(full);
            }
        }
    }

    out
}

fn contains_glob_char(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

/// Return true if any path component between `root` and `path`
/// matches a vendored / build-output directory name. The root itself
/// is NOT checked (the root can legitimately live under a dir named
/// `vendor/` on disk).
fn path_contains_excluded_dir(root: &Path, path: &Path) -> bool {
    let rel = match path.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return false,
    };
    rel.components().any(|c| {
        if let std::path::Component::Normal(name) = c {
            if let Some(s) = name.to_str() {
                return WORKSPACE_EXPANSION_EXCLUDED_DIRS.contains(&s);
            }
        }
        false
    })
}

/// Last-resort regex extractor for `key: [ "./a", "./b" ]` / block-style
/// YAML lists when `serde_yaml` rejects the document. Matches only the
/// shape real-world `pnpm-workspace.yaml` files take.
fn fallback_extract_yaml_list(content: &str, key: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_block = false;
    let prefix = format!("{}:", key);

    for line in content.lines() {
        let raw = line;
        let trimmed = line.trim_start();

        if !in_block {
            if trimmed.starts_with(&prefix) {
                // Flow-style single-line list: `packages: ["./a", "./b"]`
                if let Some(rest) = trimmed.strip_prefix(&prefix) {
                    let rest = rest.trim();
                    if let Some(inner) = rest.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                        for piece in inner.split(',') {
                            let cleaned = piece
                                .trim()
                                .trim_matches('"')
                                .trim_matches('\'')
                                .to_string();
                            if !cleaned.is_empty() {
                                out.push(cleaned);
                            }
                        }
                        return out;
                    }
                    if rest.is_empty() {
                        in_block = true;
                    }
                }
            }
            continue;
        }

        // In the block: accept `  - "./foo"` until a less-indented line.
        if raw.trim().is_empty() {
            continue;
        }
        if !raw.starts_with(' ') && !raw.starts_with('\t') {
            // Dedented past our key — block ended.
            break;
        }
        let t = raw.trim();
        if let Some(item) = t.strip_prefix("- ") {
            let cleaned = item.trim().trim_matches('"').trim_matches('\'').to_string();
            if !cleaned.is_empty() {
                out.push(cleaned);
            }
        }
    }

    out
}

/// Project-wide call graph (spec Section 2.2.1)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectCallGraph {
    edges: HashSet<CallEdge>,
}

/// Edge in the call graph
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallEdge {
    /// Path to the file containing the calling function
    pub src_file: PathBuf,
    /// Name of the calling function
    pub src_func: String,
    /// Path to the file containing the called function
    pub dst_file: PathBuf,
    /// Name of the called function
    pub dst_func: String,
}

// =============================================================================
// Type-Aware Call Graph Types (Phase 7-8: Type Resolution)
// =============================================================================

/// Confidence level for type resolution
///
/// Indicates how confident we are in the type resolution:
/// - High: Explicit annotation or constructor call
/// - Medium: Return type inference or union type
/// - Low: No type info available (fallback to variable name)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// Explicit annotation, constructor, or self/this reference
    High,
    /// Return type inference, union type, or interface
    Medium,
    /// Unknown type, fallback to variable name
    #[default]
    Low,
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Confidence::High => write!(f, "HIGH"),
            Confidence::Medium => write!(f, "MEDIUM"),
            Confidence::Low => write!(f, "LOW"),
        }
    }
}

/// Extended call edge with type resolution metadata
///
/// Used when --type-aware flag is enabled to track:
/// - The resolved receiver type (e.g., "User" instead of "user")
/// - Confidence level of the resolution
/// - Line number of the call site
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TypedCallEdge {
    /// Path to the file containing the calling function
    pub src_file: PathBuf,
    /// Name of the calling function
    pub src_func: String,
    /// Path to the file containing the called function
    pub dst_file: PathBuf,
    /// Name of the called function
    pub dst_func: String,
    /// Resolved receiver type (e.g., "User" for user.save())
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver_type: Option<String>,
    /// Confidence level of the type resolution
    pub confidence: Confidence,
    /// Line number of the call site
    pub call_site_line: u32,
}

impl TypedCallEdge {
    /// Create a new typed call edge from a basic CallEdge
    pub fn from_call_edge(edge: &CallEdge, line: u32) -> Self {
        Self {
            src_file: edge.src_file.clone(),
            src_func: edge.src_func.clone(),
            dst_file: edge.dst_file.clone(),
            dst_func: edge.dst_func.clone(),
            receiver_type: None,
            confidence: Confidence::Low,
            call_site_line: line,
        }
    }

    /// Create a high-confidence typed call edge
    pub fn high_confidence(
        src_file: PathBuf,
        src_func: String,
        dst_file: PathBuf,
        dst_func: String,
        receiver_type: String,
        line: u32,
    ) -> Self {
        Self {
            src_file,
            src_func,
            dst_file,
            dst_func,
            receiver_type: Some(receiver_type),
            confidence: Confidence::High,
            call_site_line: line,
        }
    }

    /// Create a medium-confidence typed call edge
    pub fn medium_confidence(
        src_file: PathBuf,
        src_func: String,
        dst_file: PathBuf,
        dst_func: String,
        receiver_type: String,
        line: u32,
    ) -> Self {
        Self {
            src_file,
            src_func,
            dst_file,
            dst_func,
            receiver_type: Some(receiver_type),
            confidence: Confidence::Medium,
            call_site_line: line,
        }
    }

    /// Convert to basic CallEdge (loses type info)
    pub fn to_call_edge(&self) -> CallEdge {
        CallEdge {
            src_file: self.src_file.clone(),
            src_func: self.src_func.clone(),
            dst_file: self.dst_file.clone(),
            dst_func: self.dst_func.clone(),
        }
    }
}

/// Statistics on type resolution (T17 mitigation)
///
/// Provides observability into how well type resolution worked,
/// helping users understand if --type-aware is useful for their codebase.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TypeResolutionStats {
    /// Whether type-aware analysis was enabled
    pub enabled: bool,
    /// Number of calls resolved with HIGH confidence
    pub resolved_high_confidence: usize,
    /// Number of calls resolved with MEDIUM confidence
    pub resolved_medium_confidence: usize,
    /// Number of calls that fell back to variable name (LOW confidence)
    pub fallback_used: usize,
    /// Total number of call sites analyzed
    pub total_call_sites: usize,
}

impl TypeResolutionStats {
    /// Create stats with type-aware enabled
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Default::default()
        }
    }

    /// Record a high-confidence resolution
    pub fn record_high(&mut self) {
        self.resolved_high_confidence += 1;
        self.total_call_sites += 1;
    }

    /// Record a medium-confidence resolution
    pub fn record_medium(&mut self) {
        self.resolved_medium_confidence += 1;
        self.total_call_sites += 1;
    }

    /// Record a fallback (low confidence)
    pub fn record_fallback(&mut self) {
        self.fallback_used += 1;
        self.total_call_sites += 1;
    }

    /// Get the percentage of successfully resolved calls (HIGH + MEDIUM)
    pub fn resolution_rate(&self) -> f64 {
        if self.total_call_sites == 0 {
            return 0.0;
        }
        let resolved = self.resolved_high_confidence + self.resolved_medium_confidence;
        (resolved as f64 / self.total_call_sites as f64) * 100.0
    }

    /// Format as human-readable summary
    pub fn summary(&self) -> String {
        if !self.enabled {
            return "Type resolution: disabled".to_string();
        }
        let resolved = self.resolved_high_confidence + self.resolved_medium_confidence;
        format!(
            "Type-aware resolution: {}/{} calls resolved ({} high, {} medium confidence)",
            resolved,
            self.total_call_sites,
            self.resolved_high_confidence,
            self.resolved_medium_confidence
        )
    }
}

impl ProjectCallGraph {
    /// Create a new empty call graph
    pub fn new() -> Self {
        Self {
            edges: HashSet::new(),
        }
    }

    /// Iterate over all edges
    pub fn edges(&self) -> impl Iterator<Item = &CallEdge> {
        self.edges.iter()
    }

    /// Add an edge to the graph
    pub fn add_edge(&mut self, edge: CallEdge) {
        self.edges.insert(edge);
    }

    /// Check if the graph contains an edge
    pub fn contains(&self, edge: &CallEdge) -> bool {
        self.edges.contains(edge)
    }

    /// Get the number of edges
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Check if graph is empty
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }
}

// =============================================================================
// Impact Analysis Types (spec Section 2.2.2)
// =============================================================================

/// Impact analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactReport {
    /// Map from target function name to its caller tree
    pub targets: HashMap<String, CallerTree>,
    /// Total number of target functions analyzed
    pub total_targets: usize,
    /// Type resolution statistics (when --type-aware is enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_resolution: Option<TypeResolutionStats>,
}

/// Tree of callers for impact analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallerTree {
    /// Name of the function at this node
    pub function: String,
    /// Path to the file containing this function
    pub file: PathBuf,
    /// Number of direct callers of this function
    pub caller_count: usize,
    /// Recursive tree of callers (callers of callers)
    pub callers: Vec<CallerTree>,
    /// Whether the caller tree was truncated due to depth limits
    #[serde(default)]
    pub truncated: bool,
    /// Optional note about this node (e.g., truncation reason)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Confidence of type resolution for this caller (when --type-aware is enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    /// Resolved receiver type (when --type-aware is enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver_type: Option<String>,
}

// =============================================================================
// Dead Code Types (spec Section 2.2.3)
// =============================================================================

/// Dead code analysis report
///
/// Functions are classified into two tiers:
/// - `dead_functions`: Definitely dead (private/unenriched + uncalled + no special metadata)
/// - `possibly_dead`: Public/exported but uncalled (may be API surface)
///
/// The `dead_percentage` is calculated from `dead_functions` only (definitely dead).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeReport {
    /// Functions that are definitely dead (private and uncalled)
    pub dead_functions: Vec<FunctionRef>,
    /// Public/exported functions that are uncalled (may be intentional API surface)
    #[serde(default)]
    pub possibly_dead: Vec<FunctionRef>,
    /// Map from file path to names of dead functions in that file
    pub by_file: HashMap<PathBuf, Vec<String>>,
    /// Count of definitely-dead functions
    pub total_dead: usize,
    /// Number of possibly-dead (public but uncalled) functions
    #[serde(default)]
    pub total_possibly_dead: usize,
    /// Total number of functions in the analyzed codebase
    pub total_functions: usize,
    /// Percentage of definitely-dead functions (excludes possibly_dead)
    pub dead_percentage: f64,
}

// =============================================================================
// Importers Types (spec Section 2.2.4)
// =============================================================================

/// Report of files importing a module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportersReport {
    /// Name of the module being queried
    pub module: String,
    /// Files that import this module
    pub importers: Vec<ImporterInfo>,
    /// Total number of importers found
    pub total: usize,
}

/// Information about a file that imports a module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImporterInfo {
    /// Path to the file that contains the import
    pub file: PathBuf,
    /// Line number of the import statement (1-indexed)
    pub line: u32,
    /// Full text of the import statement
    pub import_statement: String,
}

// =============================================================================
// Architecture Types (spec Section 2.2.5)
// =============================================================================

/// Architecture analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureReport {
    /// Functions in the entry layer (called by external consumers, call others)
    pub entry_layer: Vec<FunctionRef>,
    /// Functions in the middle/service layer (called by entry, call leaf)
    pub middle_layer: Vec<FunctionRef>,
    /// Functions in the leaf/utility layer (called by others, call nothing external)
    pub leaf_layer: Vec<FunctionRef>,
    /// Per-directory statistics (function counts, call directions)
    pub directories: HashMap<PathBuf, DirStats>,
    /// Detected circular dependencies between directories
    pub circular_dependencies: Vec<CircularDep>,
    /// Inferred architectural layer for each directory
    pub inferred_layers: HashMap<PathBuf, LayerType>,
}

/// Directory statistics for architecture analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirStats {
    /// Names of functions defined in this directory
    pub functions: Vec<String>,
    /// Number of outgoing calls from this directory to other directories
    pub calls_out: usize,
    /// Number of incoming calls from other directories into this directory
    pub calls_in: usize,
}

/// Circular dependency between directories
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircularDep {
    /// First directory in the circular dependency
    pub a: PathBuf,
    /// Second directory in the circular dependency
    pub b: PathBuf,
}

/// Inferred layer type for a directory
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LayerType {
    /// Entry point layer (API handlers, CLI commands, main functions)
    Entry,
    /// Service/business logic layer (orchestrates utilities)
    Service,
    /// Utility/leaf layer (pure helpers, no external dependencies)
    Utility,
    /// Dynamic dispatch layer (virtual calls, trait objects, callbacks)
    DynamicDispatch,
}

// =============================================================================
// CFG Types (Layer 3, spec Section 2.3)
// =============================================================================

/// Control flow graph information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgInfo {
    /// Name of the function this CFG represents
    pub function: String,
    /// Basic blocks in the control flow graph
    pub blocks: Vec<CfgBlock>,
    /// Edges connecting basic blocks
    pub edges: Vec<CfgEdge>,
    /// ID of the entry basic block
    pub entry_block: usize,
    /// IDs of exit basic blocks (return/end points)
    pub exit_blocks: Vec<usize>,
    /// Cyclomatic complexity of this function
    pub cyclomatic_complexity: u32,
    /// CFGs for nested/inner functions defined within this function
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub nested_functions: HashMap<String, CfgInfo>,
}

/// Basic block in CFG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgBlock {
    /// Unique identifier for this basic block
    pub id: usize,
    /// Classification of this basic block (entry, branch, loop, etc.)
    pub block_type: BlockType,
    /// Line range covered by this block (start_line, end_line), 1-indexed
    pub lines: (u32, u32),
    /// Function calls made within this basic block
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<String>,
}

/// Type of basic block
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockType {
    /// Function entry point
    Entry,
    /// Conditional branch (if/else, match)
    Branch,
    /// Loop condition check (for, while header)
    LoopHeader,
    /// Loop body statements
    LoopBody,
    /// Return statement
    Return,
    /// Function exit point
    Exit,
    /// Sequential statement block
    Body,
}

/// Edge in CFG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgEdge {
    /// ID of the source basic block
    pub from: usize,
    /// ID of the target basic block
    pub to: usize,
    /// Classification of this edge (true branch, false branch, unconditional, etc.)
    pub edge_type: EdgeType,
    /// Condition expression for conditional edges (e.g., `x > 0`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

/// Type of CFG edge
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    /// True branch of a conditional
    True,
    /// False branch of a conditional
    False,
    /// Unconditional flow (fallthrough, goto)
    Unconditional,
    /// Back edge to a loop header
    BackEdge,
    /// Break out of a loop
    Break,
    /// Continue to next loop iteration
    Continue,
}

/// Complexity metrics (spec Section 2.3.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityMetrics {
    /// Name of the function being measured
    pub function: String,
    /// Cyclomatic complexity (number of independent paths)
    pub cyclomatic: u32,
    /// Cognitive complexity (how hard the function is to understand)
    pub cognitive: u32,
    /// Maximum nesting depth of control structures
    pub nesting_depth: u32,
    /// Number of lines of code in the function
    pub lines_of_code: u32,
}

// =============================================================================
// DFG Types (Layer 4, spec Section 2.4)
// =============================================================================

/// Data flow graph information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DfgInfo {
    /// Name of the function this data flow graph represents
    pub function: String,
    /// All variable references (definitions, updates, uses) in the function
    pub refs: Vec<VarRef>,
    /// Data flow edges (def-use chains) connecting definitions to their uses
    pub edges: Vec<DataflowEdge>,
    /// Names of all variables tracked in this function
    pub variables: Vec<String>,
}

/// Variable reference in DFG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarRef {
    /// Name of the variable being referenced
    pub name: String,
    /// Whether this is a definition, update, or use of the variable
    pub ref_type: RefType,
    /// Line number of this reference (1-indexed)
    pub line: u32,
    /// Column number of this reference (0-indexed)
    pub column: u32,
    /// Language-specific construct context (e.g., "augmented_assignment", "destructuring")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<VarRefContext>,
    /// Statement group ID for parallel assignments (e.g., a, b = b, a)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<u32>,
}

/// Context for language-specific variable reference patterns
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VarRefContext {
    // Python-specific
    /// x += 1: both use and def in same statement
    AugmentedAssignment,
    /// a, b = b, a: parallel semantics (RHS evaluated before LHS)
    MultipleAssignment,
    /// n := expr: walrus operator, def in expression context
    WalrusOperator,
    /// [x for x in ...]: x is scoped to comprehension
    ComprehensionScope,
    /// match case (x, y): pattern binding
    MatchBinding,
    /// global x / nonlocal x: external scope reference
    GlobalNonlocal,

    // TypeScript/JavaScript-specific
    /// const {a, b} = obj: destructuring creates multiple defs
    Destructuring,
    /// Closure captures variable by reference
    ClosureCapture,
    /// Optional chaining (?.) short-circuit
    OptionalChain,

    // Go-specific
    /// x := 1: short declaration (may be new var or redefinition)
    ShortDeclaration,
    /// a, b := f(): multiple return values
    MultipleReturn,
    /// _ = x: blank identifier (not a real definition)
    BlankIdentifier,
    /// defer log(x): captured at defer point
    DeferCapture,

    // Rust-specific
    /// let x = 1; let x = 2: shadowing creates NEW variable
    Shadowing,
    /// let (a, b) = tuple: pattern binding
    PatternBinding,
    /// let b = a: ownership move ends a's liveness
    OwnershipMove,
    /// match x { Some(v) => ... }: binding scoped to arm
    MatchArmBinding,
}

/// Type of variable reference
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RefType {
    /// Variable definition (first assignment)
    Definition,
    /// Variable update (reassignment or mutation)
    Update,
    /// Variable use (read)
    Use,
}

/// Data flow edge (def-use chain)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataflowEdge {
    /// Name of the variable flowing from definition to use
    pub var: String,
    /// Line number where the variable is defined (1-indexed)
    pub def_line: u32,
    /// Line number where the variable is used (1-indexed)
    pub use_line: u32,
    /// Full variable reference at the definition site
    pub def_ref: VarRef,
    /// Full variable reference at the use site
    pub use_ref: VarRef,
}

// =============================================================================
// PDG Types (Layer 5, spec Section 2.5)
// =============================================================================

/// Program dependence graph information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdgInfo {
    /// Name of the function this PDG represents
    pub function: String,
    /// Control flow graph for this function
    pub cfg: CfgInfo,
    /// Data flow graph for this function
    pub dfg: DfgInfo,
    /// Nodes in the program dependence graph
    pub nodes: Vec<PdgNode>,
    /// Dependence edges (control and data) between PDG nodes
    pub edges: Vec<PdgEdge>,
}

/// Node in PDG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdgNode {
    /// Unique identifier for this PDG node
    pub id: usize,
    /// Type of statement at this node (e.g., "assignment", "branch", "call")
    pub node_type: String,
    /// Line range covered by this node (start_line, end_line), 1-indexed
    pub lines: (u32, u32),
    /// Variables defined at this node
    pub definitions: Vec<String>,
    /// Variables used at this node
    pub uses: Vec<String>,
}

/// Edge in PDG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdgEdge {
    /// ID of the source PDG node
    pub source_id: usize,
    /// ID of the target PDG node
    pub target_id: usize,
    /// Whether this is a control or data dependence
    pub dep_type: DependenceType,
    /// Human-readable label describing the dependence (e.g., variable name)
    pub label: String,
}

/// Type of dependence in PDG
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DependenceType {
    /// Control dependence (execution of target depends on a branch decision)
    Control,
    /// Data dependence (target uses a value defined by source)
    Data,
}

/// Slice direction for program slicing (spec Section 2.5.2)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SliceDirection {
    /// Backward slice: find all statements that affect the slicing criterion
    Backward,
    /// Forward slice: find all statements affected by the slicing criterion
    Forward,
}

impl std::str::FromStr for SliceDirection {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "backward" | "back" | "b" => Ok(SliceDirection::Backward),
            "forward" | "fwd" | "f" => Ok(SliceDirection::Forward),
            _ => Err(format!(
                "Invalid direction: {}. Expected 'backward' or 'forward'",
                s
            )),
        }
    }
}

/// Thin slice result (spec Section 2.5.3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinSliceResult {
    /// Line numbers in the thin (data-only) slice
    pub lines: HashSet<u32>,
    /// Line numbers in the full (data + control) slice for comparison
    pub full_slice_lines: HashSet<u32>,
    /// Percentage reduction from full slice to thin slice
    pub reduction_pct: f64,
}

// =============================================================================
// Search Types (spec Section 2.6)
// =============================================================================

/// Search match result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    /// Path to the file containing the match
    pub file: PathBuf,
    /// Line number of the match (1-indexed)
    pub line: u32,
    /// Content of the matching line
    pub content: String,
    /// Surrounding context lines (before and after the match)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<String>>,
}

/// BM25 search result (spec Section 2.6.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25Result {
    /// Path to the file containing the result
    pub file_path: PathBuf,
    /// BM25 relevance score
    pub score: f64,
    /// Start line of the matching snippet (1-indexed)
    pub line_start: u32,
    /// End line of the matching snippet (1-indexed)
    pub line_end: u32,
    /// Text snippet containing the match
    pub snippet: String,
    /// Query terms that matched in this result
    pub matched_terms: Vec<String>,
}

/// Embedding client placeholder for hybrid search
pub struct EmbeddingClient;

/// Hybrid search result (spec Section 2.6.3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridResult {
    /// Path to the file containing the result
    pub file_path: PathBuf,
    /// Reciprocal Rank Fusion score combining BM25 and dense retrieval
    pub rrf_score: f64,
    /// Rank from the BM25 retriever, if this result appeared in BM25 results
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25_rank: Option<usize>,
    /// Rank from the dense (embedding) retriever, if applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dense_rank: Option<usize>,
    /// Raw BM25 score, if this result appeared in BM25 results
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25_score: Option<f64>,
    /// Raw dense (cosine similarity) score, if applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dense_score: Option<f64>,
    /// Text snippet containing the match
    pub snippet: String,
    /// Query terms that matched in this result
    pub matched_terms: Vec<String>,
}

/// Hybrid search report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchReport {
    /// Ranked search results after reciprocal rank fusion
    pub results: Vec<HybridResult>,
    /// Original search query string
    pub query: String,
    /// Total number of candidate results before ranking
    pub total_candidates: usize,
    /// Number of results found only by BM25 (not dense retrieval)
    pub bm25_only: usize,
    /// Number of results found only by dense retrieval (not BM25)
    pub dense_only: usize,
    /// Number of results found by both retrievers
    pub overlap: usize,
    /// Fallback mode used when dense retrieval is unavailable (e.g., "bm25_only")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_mode: Option<String>,
}

// =============================================================================
// Context Types (spec Section 2.7)
// =============================================================================

/// Relevant context for LLM (spec Section 2.7.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevantContext {
    /// Name of the entry point function for context gathering
    pub entry_point: String,
    /// Maximum call depth traversed to gather context
    pub depth: usize,
    /// Functions reachable from the entry point within the specified depth
    pub functions: Vec<FunctionContext>,
}

/// Function context for LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionContext {
    /// Name of the function
    pub name: String,
    /// Path to the file containing this function
    pub file: PathBuf,
    /// Line number where the function is defined (1-indexed)
    pub line: u32,
    /// Full function signature
    pub signature: String,
    /// Docstring or doc comment, if present
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// Names of functions called by this function
    pub calls: Vec<String>,
    /// Number of basic blocks in the function's CFG
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<usize>,
    /// Cyclomatic complexity of the function
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cyclomatic: Option<u32>,
}

impl RelevantContext {
    /// Format for LLM consumption
    pub fn to_llm_string(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("# Context for: {}\n\n", self.entry_point));
        for func in &self.functions {
            output.push_str(&format!("## {}\n", func.name));
            output.push_str(&format!("File: {}:{}\n", func.file.display(), func.line));
            output.push_str(&format!("Signature: {}\n", func.signature));
            if let Some(doc) = &func.docstring {
                output.push_str(&format!("Doc: {}\n", doc));
            }
            if !func.calls.is_empty() {
                output.push_str(&format!("Calls: {}\n", func.calls.join(", ")));
            }
            output.push('\n');
        }
        output
    }
}

// =============================================================================
// Change Impact Types (spec Section 2.7.2)
// =============================================================================

/// Change impact report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeImpactReport {
    /// Files that were changed (from git diff or explicit input)
    pub changed_files: Vec<PathBuf>,
    /// Test files potentially affected by the changes
    pub affected_tests: Vec<PathBuf>,
    /// Functions transitively affected by the changes
    pub affected_functions: Vec<FunctionRef>,
    /// Method used to detect impacts (e.g., "call_graph", "import_graph")
    pub detection_method: String,
}

// =============================================================================
// Quality Types (spec Section 2.8)
// =============================================================================

/// Threshold preset for quality checks
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ThresholdPreset {
    /// Strict thresholds (lower tolerance for smells and complexity)
    Strict,
    /// Default thresholds (balanced tolerance)
    #[default]
    Default,
    /// Relaxed thresholds (higher tolerance, fewer warnings)
    Relaxed,
}

/// Code smell type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SmellType {
    /// Class that does too much (high number of methods, fields, or responsibilities)
    GodClass,
    /// Method with too many lines of code or excessive complexity
    LongMethod,
    /// Method that uses another class's data more than its own
    FeatureEnvy,
    /// Groups of fields that frequently appear together across classes
    DataClumps,
    /// Function with too many parameters
    LongParameterList,
}

/// Code smells report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmellsReport {
    /// Individual code smell findings
    pub smells: Vec<SmellFinding>,
    /// Number of files analyzed for code smells
    pub files_analyzed: usize,
    /// Total number of code smells found
    pub total_smells: usize,
}

/// Individual smell finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmellFinding {
    /// Path to the file containing the smell
    pub file: PathBuf,
    /// Line number where the smell occurs (1-indexed)
    pub line: u32,
    /// Classification of the code smell
    pub smell_type: SmellType,
    /// Human-readable description of the smell
    pub description: String,
    /// Suggested fix or refactoring
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// Maintainability report (spec Section 2.8.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintainabilityReport {
    /// Per-file maintainability index results
    pub files: Vec<FileMI>,
    /// Aggregate summary of maintainability across all files
    pub summary: MISummary,
}

/// File maintainability index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMI {
    /// Path to the source file
    pub path: PathBuf,
    /// Maintainability Index score (0-100, higher is better)
    pub mi: f64,
    /// Letter grade (A, B, or C) derived from the MI score
    pub grade: char,
    /// Halstead metrics used in MI calculation, if computed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub halstead: Option<HalsteadMetrics>,
}

/// MI summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MISummary {
    /// Average Maintainability Index across all files
    pub average_mi: f64,
    /// Lowest Maintainability Index (worst file)
    pub min_mi: f64,
    /// Highest Maintainability Index (best file)
    pub max_mi: f64,
    /// Number of files included in the summary
    pub files_analyzed: usize,
}

/// Halstead metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HalsteadMetrics {
    /// Number of distinct operators and operands (n = n1 + n2)
    pub vocabulary: u32,
    /// Total number of operators and operands (N = N1 + N2)
    pub length: u32,
    /// Volume: N * log2(n), measures information content
    pub volume: f64,
    /// Difficulty: (n1/2) * (N2/n2), measures error-proneness
    pub difficulty: f64,
    /// Effort: D * V, measures cognitive effort to understand
    pub effort: f64,
}

// =============================================================================
// Security Types (spec Section 2.9)
// =============================================================================

/// Severity level for security findings
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Low severity (informational, minor risk)
    Low,
    /// Medium severity (moderate risk, should be addressed)
    Medium,
    /// High severity (significant risk, needs prompt attention)
    High,
    /// Critical severity (immediate risk, must be fixed urgently)
    Critical,
}

/// Secrets report (spec Section 2.9.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsReport {
    /// Individual secret findings (hardcoded keys, tokens, passwords)
    pub findings: Vec<SecretFinding>,
    /// Number of files scanned for secrets
    pub files_scanned: usize,
    /// Number of secret patterns checked
    pub patterns_checked: usize,
    /// Aggregate summary of findings by severity
    pub summary: SecretsSummary,
}

/// Secret finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretFinding {
    /// Path to the file containing the secret
    pub file: PathBuf,
    /// Line number where the secret was found (1-indexed)
    pub line: u32,
    /// Name of the pattern that matched (e.g., "AWS_ACCESS_KEY")
    pub pattern: String,
    /// Severity of the finding
    pub severity: Severity,
    /// Partially masked value showing the secret type without exposing it
    pub masked_value: String,
}

/// Secrets summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsSummary {
    /// Total number of secret findings
    pub total_findings: usize,
    /// Breakdown of findings by severity level
    pub by_severity: HashMap<String, usize>,
}

/// Vulnerability type (spec Section 2.9.2)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VulnType {
    /// SQL injection via unsanitized user input in queries
    SqlInjection,
    /// Cross-site scripting via unescaped output
    Xss,
    /// OS command injection via unsanitized shell arguments
    CommandInjection,
    /// Path traversal via unvalidated file paths
    PathTraversal,
    /// Server-side request forgery via user-controlled URLs
    Ssrf,
    /// Unsafe deserialization of untrusted data
    Deserialization,
}

/// Vulnerability report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnReport {
    /// Individual vulnerability findings
    pub findings: Vec<VulnFinding>,
    /// Number of files scanned for vulnerabilities
    pub files_scanned: usize,
    /// Aggregate summary by type and severity
    pub summary: VulnSummary,
}

/// Vulnerability finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnFinding {
    /// Path to the file containing the vulnerability
    pub file: PathBuf,
    /// Line number where the vulnerability occurs (1-indexed)
    pub line: u32,
    /// Classification of the vulnerability
    pub vuln_type: VulnType,
    /// Severity of the vulnerability
    pub severity: Severity,
    /// Human-readable description of the vulnerability
    pub description: String,
    /// Taint source (where untrusted data enters), if identified
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Taint sink (where untrusted data is consumed unsafely), if identified
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sink: Option<String>,
}

/// Vulnerability summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnSummary {
    /// Total number of vulnerability findings
    pub total_findings: usize,
    /// Breakdown of findings by vulnerability type
    pub by_type: HashMap<String, usize>,
    /// Breakdown of findings by severity level
    pub by_severity: HashMap<String, usize>,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_extension() {
        assert_eq!(Language::from_extension(".py"), Some(Language::Python));
        assert_eq!(Language::from_extension(".ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension(".tsx"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension(".js"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension(".go"), Some(Language::Go));
        assert_eq!(Language::from_extension(".rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension(".java"), Some(Language::Java));
        assert_eq!(Language::from_extension(".unknown"), None);
    }

    #[test]
    fn test_language_from_extension_without_dot() {
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
    }

    #[test]
    fn test_language_from_extension_case_insensitive() {
        assert_eq!(Language::from_extension(".PY"), Some(Language::Python));
        assert_eq!(Language::from_extension(".Ts"), Some(Language::TypeScript));
    }

    #[test]
    fn test_language_serde_roundtrip() {
        for lang in Language::all() {
            let json = serde_json::to_string(lang).unwrap();
            let parsed: Language = serde_json::from_str(&json).unwrap();
            assert_eq!(*lang, parsed);
        }
    }

    #[test]
    fn test_language_all_18_variants() {
        assert_eq!(Language::all().len(), 18);
    }

    #[test]
    fn test_language_from_str() {
        assert_eq!("python".parse::<Language>().unwrap(), Language::Python);
        assert_eq!("py".parse::<Language>().unwrap(), Language::Python);
        assert_eq!(
            "typescript".parse::<Language>().unwrap(),
            Language::TypeScript
        );
        assert_eq!("ts".parse::<Language>().unwrap(), Language::TypeScript);
        assert_eq!("golang".parse::<Language>().unwrap(), Language::Go);
        assert!("unknown".parse::<Language>().is_err());
    }

    #[test]
    fn test_function_ref_equality() {
        let ref1 = FunctionRef::new(PathBuf::from("test.py"), "func");
        let ref2 = FunctionRef::new(PathBuf::from("test.py"), "func");
        let ref3 = FunctionRef::new(PathBuf::from("test.py"), "other");

        assert_eq!(ref1, ref2);
        assert_ne!(ref1, ref3);
    }

    #[test]
    fn test_function_ref_hash() {
        use std::collections::HashSet;

        let mut set = HashSet::new();
        set.insert(FunctionRef::new(PathBuf::from("test.py"), "func"));
        set.insert(FunctionRef::new(PathBuf::from("test.py"), "func"));

        assert_eq!(set.len(), 1);
    }

    #[test]
    fn test_project_call_graph() {
        let mut graph = ProjectCallGraph::new();
        assert!(graph.is_empty());
        assert_eq!(graph.edge_count(), 0);

        let edge = CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "foo".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "bar".to_string(),
        };

        graph.add_edge(edge.clone());
        assert!(!graph.is_empty());
        assert_eq!(graph.edge_count(), 1);
        assert!(graph.contains(&edge));
    }

    #[test]
    fn test_slice_direction_from_str() {
        assert_eq!(
            "backward".parse::<SliceDirection>().unwrap(),
            SliceDirection::Backward
        );
        assert_eq!(
            "forward".parse::<SliceDirection>().unwrap(),
            SliceDirection::Forward
        );
        assert_eq!(
            "back".parse::<SliceDirection>().unwrap(),
            SliceDirection::Backward
        );
        assert_eq!(
            "fwd".parse::<SliceDirection>().unwrap(),
            SliceDirection::Forward
        );
        assert!("invalid".parse::<SliceDirection>().is_err());
    }

    #[test]
    fn test_relevant_context_to_llm_string() {
        let ctx = RelevantContext {
            entry_point: "main".to_string(),
            depth: 2,
            functions: vec![FunctionContext {
                name: "main".to_string(),
                file: PathBuf::from("app.py"),
                line: 10,
                signature: "def main() -> None".to_string(),
                docstring: Some("Entry point".to_string()),
                calls: vec!["helper".to_string()],
                blocks: Some(3),
                cyclomatic: Some(2),
            }],
        };

        let output = ctx.to_llm_string();
        assert!(output.contains("Context for: main"));
        assert!(output.contains("app.py:10"));
        assert!(output.contains("def main() -> None"));
        assert!(output.contains("Entry point"));
        assert!(output.contains("Calls: helper"));
    }

    #[test]
    fn test_language_from_path_typescript() {
        let path = std::path::Path::new("src/app.ts");
        assert_eq!(Language::from_path(path), Some(Language::TypeScript));
    }

    #[test]
    fn test_language_from_path_tsx() {
        let path = std::path::Path::new("components/Button.tsx");
        assert_eq!(Language::from_path(path), Some(Language::TypeScript));
    }

    #[test]
    fn test_language_from_path_go() {
        let path = std::path::Path::new("main.go");
        assert_eq!(Language::from_path(path), Some(Language::Go));
    }

    #[test]
    fn test_language_from_path_python() {
        let path = std::path::Path::new("app.py");
        assert_eq!(Language::from_path(path), Some(Language::Python));
    }

    #[test]
    fn test_language_from_path_rust() {
        let path = std::path::Path::new("lib.rs");
        assert_eq!(Language::from_path(path), Some(Language::Rust));
    }

    #[test]
    fn test_language_from_path_ocaml() {
        let path = std::path::Path::new("lib.ml");
        assert_eq!(Language::from_path(path), Some(Language::Ocaml));
    }

    #[test]
    fn test_language_from_path_unknown() {
        let path = std::path::Path::new("readme.txt");
        assert_eq!(Language::from_path(path), None);
    }

    #[test]
    fn test_language_from_path_no_extension() {
        let path = std::path::Path::new("Makefile");
        assert_eq!(Language::from_path(path), None);
    }

    #[test]
    fn test_language_from_directory_detects_majority() {
        // Tests the *extension majority* fallback when no manifest file is
        // present. If any manifest file (tsconfig.json, Cargo.toml, etc.)
        // were present at the root or immediate subdirs, manifest priority
        // would override extension counts.
        use std::fs;
        let tmp = std::env::temp_dir().join("tldr_test_from_dir_majority");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Create 3 TypeScript files and 1 Python file (no manifests)
        fs::write(tmp.join("a.ts"), "").unwrap();
        fs::write(tmp.join("b.ts"), "").unwrap();
        fs::write(tmp.join("c.tsx"), "").unwrap();
        fs::write(tmp.join("d.py"), "").unwrap();

        let detected = Language::from_directory(&tmp);
        assert_eq!(detected, Some(Language::TypeScript));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_language_from_directory_empty_returns_none() {
        use std::fs;
        let tmp = std::env::temp_dir().join("tldr_test_from_dir_empty");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let detected = Language::from_directory(&tmp);
        assert_eq!(detected, None);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_language_from_directory_checks_subdirs() {
        use std::fs;
        let tmp = std::env::temp_dir().join("tldr_test_from_dir_subdirs");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src")).unwrap();

        // No files at top level, only in subdirectory (no manifests)
        fs::write(tmp.join("src/main.go"), "").unwrap();
        fs::write(tmp.join("src/util.go"), "").unwrap();

        let detected = Language::from_directory(&tmp);
        assert_eq!(detected, Some(Language::Go));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_language_from_directory_nonexistent_returns_none() {
        let path = std::path::Path::new("/tmp/tldr_nonexistent_dir_xyz");
        let detected = Language::from_directory(path);
        assert_eq!(detected, None);
    }

    // =========================================================================
    // VAL-002: Manifest-priority tests for Language::from_directory
    //
    // The directory-level language detector must prefer manifest files
    // (tsconfig.json, Cargo.toml, go.mod, pyproject.toml, etc.) over
    // extension majority. This fixes the pnpm-monorepo bug where
    // `tldr structure /tmp/tldr-real/dub` reported Python because
    // node_modules/.pnpm/** ships thousands of .py files from node-gyp.
    // =========================================================================

    #[test]
    fn test_from_directory_manifest_tsconfig_wins_over_python_files() {
        // tsconfig.json is a TypeScript manifest; even if there's a bait
        // node_modules/fake.py, the manifest must win. The walker also
        // skips node_modules so it should be counted-zero anyway.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        std::fs::write(dir.path().join("index.ts"), "").unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        std::fs::write(dir.path().join("node_modules/fake.py"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(
            detected,
            Some(Language::TypeScript),
            "tsconfig.json manifest must win over Python bait and the walker must skip node_modules"
        );
    }

    #[test]
    fn test_from_directory_cargo_toml_wins() {
        // Cargo.toml manifest must win over .py files scattered around.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("extra.py"), "").unwrap();
        std::fs::create_dir_all(dir.path().join("scripts")).unwrap();
        std::fs::write(dir.path().join("scripts/helper.py"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Rust));
    }

    #[test]
    fn test_from_directory_go_mod_wins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/x\n").unwrap();
        std::fs::write(dir.path().join("main.go"), "package main").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Go));
    }

    #[test]
    fn test_from_directory_pyproject_toml_wins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[project]\nname=\"x\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.py"), "def x(): pass").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Python));
    }

    #[test]
    fn test_from_directory_extension_fallback_when_no_manifest() {
        // No manifests at all -> fall back to extension majority.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "").unwrap();
        std::fs::write(dir.path().join("b.rs"), "").unwrap();
        std::fs::write(dir.path().join("c.rs"), "").unwrap();
        std::fs::write(dir.path().join("d.rs"), "").unwrap();
        std::fs::write(dir.path().join("e.rs"), "").unwrap();
        std::fs::write(dir.path().join("x.py"), "").unwrap();
        std::fs::write(dir.path().join("y.py"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(
            detected,
            Some(Language::Rust),
            "extension majority must still work when no manifest is present"
        );
    }

    #[test]
    fn test_from_directory_skips_node_modules() {
        // No manifest. Root has 2 .rs files. node_modules/stuff.py x 10.
        // The walker must skip node_modules so the count is 2 Rust vs 0 Python.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "").unwrap();
        std::fs::write(dir.path().join("b.rs"), "").unwrap();

        let nm = dir.path().join("node_modules");
        std::fs::create_dir_all(&nm).unwrap();
        for i in 0..10 {
            std::fs::write(nm.join(format!("bait_{}.py", i)), "").unwrap();
        }

        let detected = Language::from_directory(dir.path());
        assert_eq!(
            detected,
            Some(Language::Rust),
            "walker must skip node_modules even without manifest priority"
        );
    }

    #[test]
    fn test_from_directory_package_json_without_ts_dep_is_javascript() {
        // package.json present WITHOUT a typescript dep + .js files -> JavaScript.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"x","dependencies":{"express":"1.0.0"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("index.js"), "module.exports = {}").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::JavaScript));
    }

    #[test]
    fn test_from_directory_package_json_with_typescript_dep_is_typescript() {
        // package.json with "typescript" devDep should pick TypeScript even
        // when tsconfig.json is absent.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"x","devDependencies":{"typescript":"5.0.0"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("index.ts"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::TypeScript));
    }

    #[test]
    fn test_from_directory_manifest_in_subdirectory() {
        // Monorepo: Cargo.toml lives in packages/core/ (one level deep).
        // The root has no manifest but the detector should still see it.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("packages/core")).unwrap();
        std::fs::write(
            dir.path().join("packages/core/Cargo.toml"),
            "[package]\nname=\"core\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("packages/core/lib.rs"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Rust));
    }

    #[test]
    fn test_from_directory_manifest_at_root_beats_subdirectory() {
        // tsconfig.json at root should beat a Cargo.toml one level deep
        // (shallower path wins as tiebreak).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.path().join("rust_subproject")).unwrap();
        std::fs::write(
            dir.path().join("rust_subproject/Cargo.toml"),
            "[package]\nname=\"nested\"\n",
        )
        .unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(
            detected,
            Some(Language::TypeScript),
            "manifest at root must win over manifest in subdirectory"
        );
    }

    #[test]
    fn test_from_directory_gradle_kts_with_more_kotlin_than_java_is_kotlin() {
        // build.gradle.kts present. If there are more .kt files than .java,
        // we pick Kotlin; otherwise Java.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("build.gradle.kts"), "").unwrap();
        std::fs::write(dir.path().join("A.kt"), "").unwrap();
        std::fs::write(dir.path().join("B.kt"), "").unwrap();
        std::fs::write(dir.path().join("C.java"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Kotlin));
    }

    #[test]
    fn test_from_directory_gradle_kts_with_more_java_than_kotlin_is_java() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("build.gradle.kts"), "").unwrap();
        std::fs::write(dir.path().join("A.java"), "").unwrap();
        std::fs::write(dir.path().join("B.java"), "").unwrap();
        std::fs::write(dir.path().join("C.kt"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Java));
    }

    #[test]
    fn test_from_directory_pom_xml_is_java() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pom.xml"), "<project/>").unwrap();
        std::fs::write(dir.path().join("App.java"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Java));
    }

    #[test]
    fn test_from_directory_gemfile_is_ruby() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Gemfile"), "source 'x'").unwrap();
        std::fs::write(dir.path().join("app.rb"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Ruby));
    }

    #[test]
    fn test_from_directory_composer_json_is_php() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("composer.json"), "{}").unwrap();
        std::fs::write(dir.path().join("index.php"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Php));
    }

    #[test]
    fn test_from_directory_mix_exs_is_elixir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mix.exs"), "defmodule X do\nend").unwrap();
        std::fs::write(dir.path().join("lib.ex"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Elixir));
    }

    #[test]
    fn test_from_directory_package_swift_is_swift() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Package.swift"), "// swift-tools-version:5").unwrap();
        std::fs::write(dir.path().join("App.swift"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Swift));
    }

    #[test]
    fn test_from_directory_setup_py_is_python() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("setup.py"), "from setuptools import setup").unwrap();
        std::fs::write(dir.path().join("a.py"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Python));
    }

    #[test]
    fn test_from_directory_requirements_txt_is_python() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "requests==2.0").unwrap();
        std::fs::write(dir.path().join("a.py"), "").unwrap();

        let detected = Language::from_directory(dir.path());
        assert_eq!(detected, Some(Language::Python));
    }

    #[test]
    fn test_from_directory_pnpm_monorepo_with_bare_root_package_json() {
        // Simulates a dub-style pnpm monorepo:
        //   - root package.json has no typescript dep (it holds turbo/prettier)
        //   - apps/web/tsconfig.json exists (depth 2)
        //   - packages/ui/tsconfig.json exists (depth 2)
        //   - node_modules has Python bait
        // The tsconfig.json at depth 2 must beat the root package.json that
        // would otherwise return JavaScript.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"monorepo","devDependencies":{"turbo":"^1"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("prettier.config.js"), "module.exports = {}").unwrap();

        std::fs::create_dir_all(dir.path().join("apps/web")).unwrap();
        std::fs::write(dir.path().join("apps/web/tsconfig.json"), "{}").unwrap();
        std::fs::write(
            dir.path().join("apps/web/page.tsx"),
            "export default () => null",
        )
        .unwrap();

        std::fs::create_dir_all(dir.path().join("packages/ui")).unwrap();
        std::fs::write(dir.path().join("packages/ui/tsconfig.json"), "{}").unwrap();
        std::fs::write(dir.path().join("packages/ui/index.ts"), "").unwrap();

        // Python bait inside node_modules (as node-gyp ships)
        std::fs::create_dir_all(dir.path().join("node_modules/.pnpm/fake/lib")).unwrap();
        for i in 0..100 {
            std::fs::write(
                dir.path()
                    .join(format!("node_modules/.pnpm/fake/lib/{}.py", i)),
                "",
            )
            .unwrap();
        }

        let detected = Language::from_directory(dir.path());
        assert_eq!(
            detected,
            Some(Language::TypeScript),
            "pnpm monorepo with tsconfig.json in packages/*/ must detect TypeScript, not JS (from root package.json without TS dep) and not Python (from node_modules bait)"
        );
    }

    // =========================================================================
    // VAL-008: All-18-languages detection
    //
    // Every supported language must be detectable by `Language::from_directory`.
    // Prior to VAL-008, 11 of 18 were manifest-covered; the 7 extension-only
    // languages (C, Cpp, CSharp, Scala, Lua, Luau, Ocaml) relied on extension
    // majority alone and could not be distinguished from one another in the
    // presence of ambiguous headers (`.h`).
    //
    // Each per-language test uses a canonical fixture: a manifest file (when
    // one exists) plus at least one representative source file. The C/C++
    // tie-break is exercised separately.
    // =========================================================================

    // ---- 11 already-covered languages (verify coverage didn't regress) ------

    #[test]
    fn test_from_directory_detects_python() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[project]\nname=\"x\"\n").unwrap();
        std::fs::write(dir.path().join("main.py"), "def x(): pass\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Python));
    }

    #[test]
    fn test_from_directory_detects_typescript() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        std::fs::write(dir.path().join("index.ts"), "export const x: number = 1;\n").unwrap();
        assert_eq!(
            Language::from_directory(dir.path()),
            Some(Language::TypeScript)
        );
    }

    #[test]
    fn test_from_directory_detects_javascript() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"x","dependencies":{"express":"1.0.0"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("index.js"), "export const x = 1;\n").unwrap();
        assert_eq!(
            Language::from_directory(dir.path()),
            Some(Language::JavaScript)
        );
    }

    #[test]
    fn test_from_directory_detects_go() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/x\ngo 1.21\n").unwrap();
        std::fs::write(dir.path().join("main.go"), "package main\nfunc main() {}\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Go));
    }

    #[test]
    fn test_from_directory_detects_rust() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "pub fn x() {}\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Rust));
    }

    #[test]
    fn test_from_directory_detects_java() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pom.xml"), "<project/>").unwrap();
        std::fs::write(
            dir.path().join("App.java"),
            "class App { public static void main(String[] a){} }\n",
        )
        .unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Java));
    }

    #[test]
    fn test_from_directory_detects_kotlin() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("build.gradle.kts"), "").unwrap();
        std::fs::write(dir.path().join("App.kt"), "fun main() {}\n").unwrap();
        std::fs::write(dir.path().join("Util.kt"), "fun util() {}\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Kotlin));
    }

    #[test]
    fn test_from_directory_detects_ruby() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Gemfile"),
            "source 'https://rubygems.org'\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("app.rb"), "def x; end\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Ruby));
    }

    #[test]
    fn test_from_directory_detects_php() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("composer.json"), r#"{"name":"x/y"}"#).unwrap();
        std::fs::write(dir.path().join("index.php"), "<?php function x(){}\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Php));
    }

    #[test]
    fn test_from_directory_detects_elixir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("mix.exs"),
            "defmodule X.MixProject do\nend\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("lib.ex"),
            "defmodule X do\ndef y(), do: :ok\nend\n",
        )
        .unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Elixir));
    }

    #[test]
    fn test_from_directory_detects_swift() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Package.swift"),
            "// swift-tools-version:5.5\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("App.swift"), "func x(){}\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Swift));
    }

    // ---- 7 newly covered languages (VAL-008) --------------------------------

    #[test]
    fn test_from_directory_detects_csharp() {
        // .csproj manifest wins over a larger pile of Python bait.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("App.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk\"/>",
        )
        .unwrap();
        // Bait: many more .py files than .cs files. Manifest must win.
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        std::fs::write(
            dir.path().join("Program.cs"),
            "class X { static void Main(){} }\n",
        )
        .unwrap();
        assert_eq!(
            Language::from_directory(dir.path()),
            Some(Language::CSharp),
            ".csproj manifest must win over .py bait"
        );
    }

    #[test]
    fn test_from_directory_detects_csharp_sln() {
        // Standalone .sln file detects CSharp over Python bait.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("App.sln"),
            "Microsoft Visual Studio Solution\n",
        )
        .unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        std::fs::write(dir.path().join("Program.cs"), "class X {}\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::CSharp));
    }

    #[test]
    fn test_from_directory_detects_csharp_global_json_with_sdk() {
        // global.json with "sdk" key = .NET SDK pin = CSharp, wins over bait.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("global.json"),
            r#"{"sdk":{"version":"8.0.100"}}"#,
        )
        .unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        std::fs::write(dir.path().join("Program.cs"), "class X {}\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::CSharp));
    }

    #[test]
    fn test_from_directory_global_json_without_sdk_is_not_csharp() {
        // global.json without "sdk" key is some other tool's config and
        // must NOT be treated as a CSharp marker.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("global.json"),
            r#"{"name":"something-else"}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("a.py"), "def x(): pass\n").unwrap();
        assert_eq!(
            Language::from_directory(dir.path()),
            Some(Language::Python),
            "unrelated global.json must not make this CSharp"
        );
    }

    #[test]
    fn test_from_directory_detects_scala() {
        // build.sbt wins over Python bait.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("build.sbt"),
            "name := \"x\"\nscalaVersion := \"3.3.0\"\n",
        )
        .unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        std::fs::write(
            dir.path().join("Main.scala"),
            "object X { def main(args: Array[String]): Unit = {} }\n",
        )
        .unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Scala));
    }

    #[test]
    fn test_from_directory_detects_scala_project_build_properties() {
        // project/build.properties is nested — exercise the sub-check.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("project")).unwrap();
        std::fs::write(
            dir.path().join("project/build.properties"),
            "sbt.version=1.9.0\n",
        )
        .unwrap();
        // Bait: more .py at root than .scala.
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        std::fs::write(
            dir.path().join("Main.scala"),
            "object X { def main(args: Array[String]): Unit = {} }\n",
        )
        .unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Scala));
    }

    #[test]
    fn test_from_directory_detects_ocaml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("dune-project"), "(lang dune 3.0)\n").unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        std::fs::write(dir.path().join("lib.ml"), "let x () = ()\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Ocaml));
    }

    #[test]
    fn test_from_directory_detects_ocaml_opam() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("my-proj.opam"), "opam-version: \"2.0\"\n").unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        std::fs::write(dir.path().join("lib.ml"), "let x () = ()\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Ocaml));
    }

    #[test]
    fn test_from_directory_detects_lua() {
        // *.rockspec wins over Python bait.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("x-1.0-1.rockspec"),
            "package = \"x\"\nversion = \"1.0-1\"\n",
        )
        .unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        std::fs::write(dir.path().join("init.lua"), "function x() end\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Lua));
    }

    #[test]
    fn test_from_directory_detects_lua_luarc() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".luarc.json"), "{}").unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        std::fs::write(dir.path().join("init.lua"), "function x() end\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Lua));
    }

    #[test]
    fn test_from_directory_detects_luau() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("default.project.json"),
            r#"{"name":"x","tree":{"$className":"DataModel"}}"#,
        )
        .unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        std::fs::write(dir.path().join("init.luau"), "function x() end\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Luau));
    }

    #[test]
    fn test_from_directory_detects_luau_luaurc() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".luaurc"), "{}").unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        std::fs::write(dir.path().join("init.luau"), "function x() end\n").unwrap();
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Luau));
    }

    // ---- C / C++ tie-break --------------------------------------------------

    #[test]
    fn test_from_directory_detects_c_with_cmake() {
        // CMakeLists.txt + .c files + Python bait → C.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3.10)\nproject(x C)\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("main.c"), "int main(){return 0;}\n").unwrap();
        std::fs::write(dir.path().join("util.c"), "int util(){return 0;}\n").unwrap();
        std::fs::write(dir.path().join("main.h"), "int main(void);\n").unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        assert_eq!(Language::from_directory(dir.path()), Some(Language::C));
    }

    #[test]
    fn test_from_directory_detects_cpp_with_cmake() {
        // CMakeLists.txt + .cpp files + Python bait → Cpp (tie-break).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3.10)\nproject(x CXX)\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("main.cpp"), "int main(){return 0;}\n").unwrap();
        std::fs::write(dir.path().join("util.cpp"), "int util(){return 0;}\n").unwrap();
        std::fs::write(dir.path().join("util.cc"), "int util2(){return 0;}\n").unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Cpp));
    }

    #[test]
    fn test_from_directory_c_project_pure_extensions() {
        // No manifest, only .c + .h → C (extension-majority fallback).
        let dir = tempfile::tempdir().unwrap();
        for name in &["a.c", "b.c", "c.c"] {
            std::fs::write(dir.path().join(name), "int x(){return 0;}\n").unwrap();
        }
        for name in &["a.h", "b.h"] {
            std::fs::write(dir.path().join(name), "int x(void);\n").unwrap();
        }
        assert_eq!(Language::from_directory(dir.path()), Some(Language::C));
    }

    #[test]
    fn test_from_directory_cpp_project_pure_extensions() {
        // No manifest, only .cpp + .h → Cpp (extension-majority fallback).
        // Note: .h → C per from_extension, but .cpp outnumbers .h (3 vs 2)
        // so extension majority is Cpp.
        let dir = tempfile::tempdir().unwrap();
        for name in &["a.cpp", "b.cpp", "c.cpp"] {
            std::fs::write(dir.path().join(name), "int x(){return 0;}\n").unwrap();
        }
        for name in &["a.h", "b.h"] {
            std::fs::write(dir.path().join(name), "int x();\n").unwrap();
        }
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Cpp));
    }

    #[test]
    fn test_from_directory_cpp_with_cmakelists_ties_break_to_cpp() {
        // CMakeLists.txt shared — tie-break by cpp-family count > c-family.
        // Adding .py bait to prove the manifest (not extension majority) is
        // responsible for picking Cpp.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CMakeLists.txt"), "project(x)\n").unwrap();
        for name in &["a.cpp", "b.cpp", "c.cpp"] {
            std::fs::write(dir.path().join(name), "int x(){return 0;}\n").unwrap();
        }
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        assert_eq!(Language::from_directory(dir.path()), Some(Language::Cpp));
    }

    #[test]
    fn test_from_directory_c_with_cmakelists_ties_break_to_c() {
        // CMakeLists.txt shared — only .c files present, pick C.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CMakeLists.txt"), "project(x)\n").unwrap();
        for name in &["a.c", "b.c", "c.c"] {
            std::fs::write(dir.path().join(name), "int x(){return 0;}\n").unwrap();
        }
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        assert_eq!(Language::from_directory(dir.path()), Some(Language::C));
    }

    #[test]
    fn test_from_directory_cpp_with_h_headers_not_misdetected_as_c() {
        // .h files outnumber .cpp sources, but the cpp-family count
        // (.cpp alone = 2) still beats the c-family count (.c = 0),
        // so this should remain Cpp — not be misdetected as C because
        // of its headers. This is the ".h ambiguity" scenario from VAL-008.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CMakeLists.txt"), "project(x)\n").unwrap();
        for name in &["a.cpp", "b.cpp"] {
            std::fs::write(dir.path().join(name), "int x(){return 0;}\n").unwrap();
        }
        for name in &["a.h", "b.h", "c.h", "d.h"] {
            std::fs::write(dir.path().join(name), "int x();\n").unwrap();
        }
        assert_eq!(
            Language::from_directory(dir.path()),
            Some(Language::Cpp),
            ".h headers must not cause a Cpp project to misdetect as C"
        );
    }

    #[test]
    fn test_from_directory_detects_c_with_meson() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("meson.build"), "project('x', 'c')\n").unwrap();
        std::fs::write(dir.path().join("main.c"), "int main(){return 0;}\n").unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        assert_eq!(Language::from_directory(dir.path()), Some(Language::C));
    }

    #[test]
    fn test_from_directory_detects_c_with_configure_ac() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("configure.ac"), "AC_INIT([x], [1.0])\n").unwrap();
        std::fs::write(dir.path().join("main.c"), "int main(){return 0;}\n").unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        assert_eq!(Language::from_directory(dir.path()), Some(Language::C));
    }

    #[test]
    fn test_from_directory_detects_c_with_makefile_am() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Makefile.am"),
            "bin_PROGRAMS = x\nx_SOURCES = main.c\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("main.c"), "int main(){return 0;}\n").unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("bait_{}.py", i)), "").unwrap();
        }
        assert_eq!(Language::from_directory(dir.path()), Some(Language::C));
    }

    // =========================================================================
    // VAL-007: WorkspaceConfig::discover
    // =========================================================================

    /// Canonical path comparison helper — tempdirs under `/var/folders` get
    /// canonicalized to `/private/var/folders` on macOS, which breaks naive
    /// PathBuf::contains checks.
    fn canon(p: &std::path::Path) -> PathBuf {
        dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    }

    fn roots_contain(ws: &WorkspaceConfig, needle: &std::path::Path) -> bool {
        let target = canon(needle);
        ws.roots.iter().any(|r| canon(r) == target)
    }

    #[test]
    fn test_discover_pnpm_workspace_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(
            root.join("pnpm-workspace.yaml"),
            "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
        )
        .unwrap();

        for sub in ["apps/foo", "apps/bar", "packages/util"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }

        let ws = WorkspaceConfig::discover(root)
            .expect("pnpm-workspace.yaml with real members should yield Some");

        assert!(
            roots_contain(&ws, root),
            "root itself should be first entry"
        );
        for sub in ["apps/foo", "apps/bar", "packages/util"] {
            assert!(
                roots_contain(&ws, &root.join(sub)),
                "expected {} among roots, got: {:?}",
                sub,
                ws.roots,
            );
        }
    }

    #[test]
    fn test_discover_package_json_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(
            root.join("package.json"),
            r#"{"name":"monorepo","workspaces":["apps/*"]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("apps/web")).unwrap();
        std::fs::create_dir_all(root.join("apps/admin")).unwrap();

        let ws = WorkspaceConfig::discover(root).expect("npm/yarn workspaces should yield Some");
        assert!(roots_contain(&ws, root));
        assert!(roots_contain(&ws, &root.join("apps/web")));
        assert!(roots_contain(&ws, &root.join("apps/admin")));
    }

    #[test]
    fn test_discover_package_json_workspaces_object_form() {
        // yarn v2/berry form: `"workspaces": { "packages": [...] }`
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(
            root.join("package.json"),
            r#"{"name":"x","workspaces":{"packages":["pkg/*"]}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("pkg/a")).unwrap();

        let ws = WorkspaceConfig::discover(root).expect("yarn berry workspaces form should work");
        assert!(roots_contain(&ws, &root.join("pkg/a")));
    }

    #[test]
    fn test_discover_cargo_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"a\", \"b\"]\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        std::fs::write(root.join("a/Cargo.toml"), "[package]\nname = \"a\"\n").unwrap();
        std::fs::write(root.join("b/Cargo.toml"), "[package]\nname = \"b\"\n").unwrap();

        let ws = WorkspaceConfig::discover(root).expect("Cargo [workspace] should yield Some");
        assert!(roots_contain(&ws, root));
        assert!(roots_contain(&ws, &root.join("a")));
        assert!(roots_contain(&ws, &root.join("b")));
    }

    #[test]
    fn test_discover_cargo_workspace_multiline_members() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\n    \"a\",\n    \"b\",\n]\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();

        let ws = WorkspaceConfig::discover(root).expect("multi-line members array should parse");
        assert!(roots_contain(&ws, &root.join("a")));
        assert!(roots_contain(&ws, &root.join("b")));
    }

    #[test]
    fn test_discover_go_work() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(root.join("go.work"), "go 1.21\n\nuse ./foo\nuse ./bar\n").unwrap();
        std::fs::create_dir_all(root.join("foo")).unwrap();
        std::fs::create_dir_all(root.join("bar")).unwrap();

        let ws = WorkspaceConfig::discover(root).expect("go.work should yield Some");
        assert!(roots_contain(&ws, &root.join("foo")));
        assert!(roots_contain(&ws, &root.join("bar")));
    }

    #[test]
    fn test_discover_go_work_block_form() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(
            root.join("go.work"),
            "go 1.21\n\nuse (\n\t./foo\n\t./bar\n)\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("foo")).unwrap();
        std::fs::create_dir_all(root.join("bar")).unwrap();

        let ws = WorkspaceConfig::discover(root).expect("go.work block form should yield Some");
        assert!(roots_contain(&ws, &root.join("foo")));
        assert!(roots_contain(&ws, &root.join("bar")));
    }

    #[test]
    fn test_discover_returns_none_on_bare_dir() {
        let dir = tempfile::tempdir().unwrap();
        // No workspace manifest present at all.
        assert!(WorkspaceConfig::discover(dir.path()).is_none());
    }

    #[test]
    fn test_discover_returns_none_when_only_root_matches() {
        // pnpm manifest exists but references nonexistent members -> None
        // (we only return Some when at least one real member is found).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(
            root.join("pnpm-workspace.yaml"),
            "packages:\n  - 'does-not-exist/*'\n",
        )
        .unwrap();

        assert!(
            WorkspaceConfig::discover(root).is_none(),
            "empty expansion should behave like a bare dir"
        );
    }

    #[test]
    fn test_discover_ignores_non_existent_members() {
        // pnpm manifest has a mix of real and missing members; only real
        // members should appear in the output, and we should not crash.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(
            root.join("pnpm-workspace.yaml"),
            "packages:\n  - 'apps/*'\n  - 'gone/*'\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("apps/real")).unwrap();

        let ws = WorkspaceConfig::discover(root).expect("one real member is enough to yield Some");
        assert!(roots_contain(&ws, &root.join("apps/real")));
        // Missing `gone/*` must not appear.
        for r in &ws.roots {
            assert!(
                !r.to_string_lossy().contains("gone"),
                "missing members should be skipped, got: {:?}",
                r,
            );
        }
    }

    #[test]
    fn test_discover_skips_node_modules_during_glob_expansion() {
        // If a workspace glob would match into node_modules/, we should
        // skip those matches. Prevents scanning 10k+ npm packages.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(root.join("pnpm-workspace.yaml"), "packages:\n  - '*'\n").unwrap();
        std::fs::create_dir_all(root.join("real-pkg")).unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();

        let ws = WorkspaceConfig::discover(root)
            .expect("should find real-pkg even when node_modules exists");
        assert!(roots_contain(&ws, &root.join("real-pkg")));
        for r in &ws.roots {
            assert!(
                !r.to_string_lossy().contains("node_modules"),
                "node_modules must not be expanded, got: {:?}",
                r,
            );
        }
    }

    #[test]
    fn test_discover_flow_style_yaml() {
        // serde_yaml should handle flow-style lists; but confirm the
        // fallback regex also handles them when present.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(
            root.join("pnpm-workspace.yaml"),
            "packages: ['./apps/a', './apps/b']\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("apps/a")).unwrap();
        std::fs::create_dir_all(root.join("apps/b")).unwrap();

        let ws = WorkspaceConfig::discover(root).expect("flow-style yaml should parse");
        assert!(roots_contain(&ws, &root.join("apps/a")));
        assert!(roots_contain(&ws, &root.join("apps/b")));
    }
}
