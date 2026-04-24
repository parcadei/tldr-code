//! VAL-010: 13 commands × 18 languages integration audit.
//!
//! This is an **audit-only** matrix that runs each of 13 representative
//! `tldr` commands against a canonical minimal fixture for each of the 18
//! supported languages. 13 × 18 = **234 test cases**, each named
//! `test_<command>_on_<language>` so a failure immediately identifies the
//! broken pair.
//!
//! # Fixture invariants
//!
//! Each fixture (see `fixtures/mod.rs`) contains:
//! - A canonical manifest matching `Language::from_directory`'s precedence.
//! - File A defining `helper` (constant return) and `main` (calls both).
//! - File B defining `b_util` (constant return), imported from File A.
//! - Exactly 2 call edges: `main -> helper`, `main -> b_util`.
//!
//! # Assertions per cell
//!
//! 1. **Exit code 0** for the informational commands.
//! 2. **JSON output parses** (some commands emit helper progress to stdout
//!    which we strip).
//! 3. **Semantic sanity** — shape-specific checks defined per command.
//!
//! # Known capability gaps
//!
//! Any (command × language) pair that cannot pass because of a documented
//! tldr-core capability gap is marked `#[ignore = "..."]` with a reason
//! pointing at the specific file:line where support ends. Silently passing
//! a broken pair is forbidden (gaming rule).
//!
//! # How to add a new known gap
//!
//! 1. Run the matrix.
//! 2. For each failure, briefly investigate the root cause in tldr-core.
//! 3. If it's a capability gap (not a fixture bug), change the test's
//!    attribute from `#[test]` to `#[test]\n#[ignore = "<reason>"]` and
//!    append a bullet to `KNOWN_CAPABILITY_GAPS` below.
//!
//! # Known capability gaps catalog (as of 2026-04-24)
//!
//! Documented when first discovered in VAL-010. Each `#[ignore]` attribute
//! MUST cite one of these. Fixing any of these gaps is a future milestone.
//!
//! - see individual test `#[ignore]` reasons for file:line citations.

mod fixtures;

use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

// ============================================================================
// Test harness
// ============================================================================

/// Get the path to the tldr binary under test.
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

/// Run a tldr command and return (exit status, parsed JSON).
///
/// Strips any leading non-JSON lines (some commands like `references` emit
/// a progress line before JSON output). If parsing fails, returns
/// `Value::Null`. Caller is responsible for verifying success.
fn run_tldr(args: &[&str]) -> (std::process::ExitStatus, Value, String, String) {
    let mut cmd = tldr_cmd();
    cmd.args(args);
    let output = cmd.output().expect("spawn tldr");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Some commands emit helper progress before JSON. Find the first `{` or `[`
    // to extract JSON.
    let json_start = stdout
        .find(['{', '['])
        .unwrap_or(stdout.len());
    let json_slice = &stdout[json_start..];
    let json = serde_json::from_str::<Value>(json_slice).unwrap_or(Value::Null);

    (output.status, json, stdout, stderr)
}

/// Panic with detailed context if a matrix assertion fails.
fn fail_cell(cmd: &str, lang: &str, why: &str, stdout: &str, stderr: &str) -> ! {
    panic!(
        "\n[{cmd} × {lang}] {why}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}\n"
    );
}

/// Return the canonical File A path within the fixture (the entry file that
/// defines `helper` and `main`). Used for file-level commands like `extract`,
/// `imports`, `complexity`, `cognitive`, `halstead`.
fn entry_file(lang: &str, root: &Path) -> std::path::PathBuf {
    match lang {
        "python" => root.join("main.py"),
        "typescript" => root.join("main.ts"),
        "javascript" => root.join("main.js"),
        "go" => root.join("main.go"),
        "rust" => root.join("src/main.rs"),
        "java" => root.join("Main.java"),
        "c" => root.join("main.c"),
        "cpp" => root.join("main.cpp"),
        "ruby" => root.join("main.rb"),
        "kotlin" => root.join("Main.kt"),
        "swift" => root.join("Main.swift"),
        "csharp" => root.join("Program.cs"),
        "scala" => root.join("Main.scala"),
        "php" => root.join("main.php"),
        "lua" => root.join("main.lua"),
        "luau" => root.join("main.luau"),
        "elixir" => root.join("main.ex"),
        "ocaml" => root.join("main.ml"),
        _ => panic!("unknown lang: {lang}"),
    }
}

// ============================================================================
// Per-command assertions
// ============================================================================

fn check_structure(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let (status, json, stdout, stderr) = run_tldr(&[
        "structure",
        tmp.path().to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("structure", lang, "non-zero exit", &stdout, &stderr);
    }
    let actual_lang = json
        .get("language")
        .and_then(Value::as_str)
        .unwrap_or("<none>");
    if actual_lang != lang {
        fail_cell(
            "structure",
            lang,
            &format!("language field was {:?}, expected {:?}", actual_lang, lang),
            &stdout,
            &stderr,
        );
    }
    let files = json.get("files").and_then(Value::as_array);
    let has_any_fn_or_class = files
        .map(|arr| {
            arr.iter().any(|f| {
                let fns = f
                    .get("functions")
                    .and_then(Value::as_array)
                    .map(|a| !a.is_empty())
                    .unwrap_or(false);
                let cls = f
                    .get("classes")
                    .and_then(Value::as_array)
                    .map(|a| !a.is_empty())
                    .unwrap_or(false);
                let methods = f
                    .get("methods")
                    .and_then(Value::as_array)
                    .map(|a| !a.is_empty())
                    .unwrap_or(false);
                fns || cls || methods
            })
        })
        .unwrap_or(false);
    if !has_any_fn_or_class {
        fail_cell(
            "structure",
            lang,
            "no files with non-empty functions/classes/methods",
            &stdout,
            &stderr,
        );
    }
}

fn check_extract(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let file = entry_file(lang, tmp.path());
    let (status, json, stdout, stderr) = run_tldr(&[
        "extract",
        file.to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("extract", lang, "non-zero exit", &stdout, &stderr);
    }
    // extract returns functions/classes/methods at top level. Accept either.
    let funcs = json
        .get("functions")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    let classes = json
        .get("classes")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    if funcs + classes == 0 {
        fail_cell(
            "extract",
            lang,
            "no functions or classes extracted",
            &stdout,
            &stderr,
        );
    }
}

fn check_imports(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let file = entry_file(lang, tmp.path());
    let (status, json, stdout, stderr) = run_tldr(&[
        "imports",
        file.to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("imports", lang, "non-zero exit", &stdout, &stderr);
    }
    // Expect a JSON array (possibly empty). We only check exit + parse.
    // Some languages' entry files don't use imports (e.g. Swift or small
    // Lua fixtures using require). Accept both empty and non-empty arrays;
    // the semantic check is that JSON parses.
    if !json.is_array() && !json.is_object() {
        fail_cell(
            "imports",
            lang,
            "output is neither JSON array nor object",
            &stdout,
            &stderr,
        );
    }
}

fn check_loc(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let (status, json, stdout, stderr) = run_tldr(&[
        "loc",
        tmp.path().to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("loc", lang, "non-zero exit", &stdout, &stderr);
    }
    let summary = json.get("summary");
    let total = summary
        .and_then(|s| s.get("total_lines"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let code = summary
        .and_then(|s| s.get("code_lines"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if total == 0 || code == 0 {
        fail_cell(
            "loc",
            lang,
            &format!("total_lines={total} code_lines={code}"),
            &stdout,
            &stderr,
        );
    }
}

fn check_complexity(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let file = entry_file(lang, tmp.path());
    // All fixtures define `helper` as the per-language function name.
    // Swift/Kotlin/Elixir accept the bare name via tree-sitter extraction.
    let (status, json, stdout, stderr) = run_tldr(&[
        "complexity",
        file.to_str().unwrap(),
        "helper",
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("complexity", lang, "non-zero exit", &stdout, &stderr);
    }
    // Accept either `cyclomatic` (newer) or `complexity` key.
    let cx = json
        .get("cyclomatic")
        .or_else(|| json.get("complexity"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if cx == 0 {
        fail_cell(
            "complexity",
            lang,
            "cyclomatic/complexity field missing or zero (expected >= 1)",
            &stdout,
            &stderr,
        );
    }
}

fn check_cognitive(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let file = entry_file(lang, tmp.path());
    let (status, json, stdout, stderr) = run_tldr(&[
        "cognitive",
        file.to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("cognitive", lang, "non-zero exit", &stdout, &stderr);
    }
    let functions = json.get("functions").and_then(Value::as_array);
    if functions.map(|a| a.is_empty()).unwrap_or(true) {
        fail_cell(
            "cognitive",
            lang,
            "empty `functions` array (no functions analyzed)",
            &stdout,
            &stderr,
        );
    }
}

fn check_halstead(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let file = entry_file(lang, tmp.path());
    let (status, json, stdout, stderr) = run_tldr(&[
        "halstead",
        file.to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("halstead", lang, "non-zero exit", &stdout, &stderr);
    }
    let functions = json.get("functions").and_then(Value::as_array);
    if functions.map(|a| a.is_empty()).unwrap_or(true) {
        fail_cell(
            "halstead",
            lang,
            "empty `functions` array (no metrics computed)",
            &stdout,
            &stderr,
        );
    }
}

fn check_smells(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let (status, json, stdout, stderr) = run_tldr(&[
        "smells",
        tmp.path().to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("smells", lang, "non-zero exit", &stdout, &stderr);
    }
    // files_scanned >= 1 OR non-empty smells array.
    let files_scanned = json
        .get("files_scanned")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let smells_arr = json
        .get("smells")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    if files_scanned == 0 && smells_arr == 0 {
        fail_cell(
            "smells",
            lang,
            "files_scanned=0 and smells is empty — pipeline ran on nothing",
            &stdout,
            &stderr,
        );
    }
}

fn check_calls(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let (status, json, stdout, stderr) = run_tldr(&[
        "calls",
        tmp.path().to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("calls", lang, "non-zero exit", &stdout, &stderr);
    }
    let total = json
        .get("total_edges")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if total == 0 {
        fail_cell(
            "calls",
            lang,
            "total_edges=0 but fixture has main -> helper and main -> b_util",
            &stdout,
            &stderr,
        );
    }
}

fn check_dead(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let (status, json, stdout, stderr) = run_tldr(&[
        "dead",
        tmp.path().to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("dead", lang, "non-zero exit", &stdout, &stderr);
    }
    // Minimum sanity: total_functions > 0 (pipeline saw *some* function).
    let total_fns = json
        .get("total_functions")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if total_fns == 0 {
        fail_cell(
            "dead",
            lang,
            "total_functions=0 — dead-code analyzer saw no functions",
            &stdout,
            &stderr,
        );
    }
}

fn check_references(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let (status, json, stdout, stderr) = run_tldr(&[
        "references",
        "helper",
        tmp.path().to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("references", lang, "non-zero exit", &stdout, &stderr);
    }
    // total_references >= 1 (definition counts as a ref in tldr's output).
    let total = json
        .get("total_references")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if total == 0 {
        fail_cell(
            "references",
            lang,
            "total_references=0 — not even the definition was found",
            &stdout,
            &stderr,
        );
    }
}

fn check_impact(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let (status, json, stdout, stderr) = run_tldr(&[
        "impact",
        "helper",
        tmp.path().to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("impact", lang, "non-zero exit", &stdout, &stderr);
    }
    let total_targets = json
        .get("total_targets")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if total_targets == 0 {
        fail_cell(
            "impact",
            lang,
            "total_targets=0 — function `helper` not located at all",
            &stdout,
            &stderr,
        );
    }
}

fn check_patterns(lang: &str) {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    let (status, json, stdout, stderr) = run_tldr(&[
        "patterns",
        tmp.path().to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    if !status.success() {
        fail_cell("patterns", lang, "non-zero exit", &stdout, &stderr);
    }
    let files_analyzed = json
        .get("metadata")
        .and_then(|m| m.get("files_analyzed"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if files_analyzed == 0 {
        fail_cell(
            "patterns",
            lang,
            "metadata.files_analyzed=0 — pipeline saw no files",
            &stdout,
            &stderr,
        );
    }
}

// ============================================================================
// 234 tests: 13 commands × 18 languages
// ============================================================================
//
// Organized command-first to make failure clusters easy to spot.

// ---------------------------------------------------------------------------- structure
#[test] fn test_structure_on_python() { check_structure("python"); }
#[test] fn test_structure_on_typescript() { check_structure("typescript"); }
#[test] fn test_structure_on_javascript() { check_structure("javascript"); }
#[test] fn test_structure_on_go() { check_structure("go"); }
#[test] fn test_structure_on_rust() { check_structure("rust"); }
#[test] fn test_structure_on_java() { check_structure("java"); }
#[test] fn test_structure_on_c() { check_structure("c"); }
#[test] fn test_structure_on_cpp() { check_structure("cpp"); }
#[test] fn test_structure_on_ruby() { check_structure("ruby"); }
#[test] fn test_structure_on_kotlin() { check_structure("kotlin"); }
#[test] fn test_structure_on_swift() { check_structure("swift"); }
#[test] fn test_structure_on_csharp() { check_structure("csharp"); }
#[test] fn test_structure_on_scala() { check_structure("scala"); }
#[test] fn test_structure_on_php() { check_structure("php"); }
#[test] fn test_structure_on_lua() { check_structure("lua"); }
#[test] fn test_structure_on_luau() { check_structure("luau"); }
#[test] fn test_structure_on_elixir() { check_structure("elixir"); }
#[test] fn test_structure_on_ocaml() { check_structure("ocaml"); }

// ---------------------------------------------------------------------------- extract
#[test] fn test_extract_on_python() { check_extract("python"); }
#[test] fn test_extract_on_typescript() { check_extract("typescript"); }
#[test] fn test_extract_on_javascript() { check_extract("javascript"); }
#[test] fn test_extract_on_go() { check_extract("go"); }
#[test] fn test_extract_on_rust() { check_extract("rust"); }
#[test] fn test_extract_on_java() { check_extract("java"); }
#[test] fn test_extract_on_c() { check_extract("c"); }
#[test] fn test_extract_on_cpp() { check_extract("cpp"); }
#[test] fn test_extract_on_ruby() { check_extract("ruby"); }
#[test] fn test_extract_on_kotlin() { check_extract("kotlin"); }
#[test] fn test_extract_on_swift() { check_extract("swift"); }
#[test] fn test_extract_on_csharp() { check_extract("csharp"); }
#[test] fn test_extract_on_scala() { check_extract("scala"); }
#[test] fn test_extract_on_php() { check_extract("php"); }
#[test] fn test_extract_on_lua() { check_extract("lua"); }
#[test] fn test_extract_on_luau() { check_extract("luau"); }
#[test] fn test_extract_on_elixir() { check_extract("elixir"); }
#[test] fn test_extract_on_ocaml() { check_extract("ocaml"); }

// ---------------------------------------------------------------------------- imports
#[test] fn test_imports_on_python() { check_imports("python"); }
#[test] fn test_imports_on_typescript() { check_imports("typescript"); }
#[test] fn test_imports_on_javascript() { check_imports("javascript"); }
#[test] fn test_imports_on_go() { check_imports("go"); }
#[test] fn test_imports_on_rust() { check_imports("rust"); }
#[test] fn test_imports_on_java() { check_imports("java"); }
#[test] fn test_imports_on_c() { check_imports("c"); }
#[test] fn test_imports_on_cpp() { check_imports("cpp"); }
#[test] fn test_imports_on_ruby() { check_imports("ruby"); }
#[test] fn test_imports_on_kotlin() { check_imports("kotlin"); }
#[test] fn test_imports_on_swift() { check_imports("swift"); }
#[test] fn test_imports_on_csharp() { check_imports("csharp"); }
#[test] fn test_imports_on_scala() { check_imports("scala"); }
#[test] fn test_imports_on_php() { check_imports("php"); }
#[test] fn test_imports_on_lua() { check_imports("lua"); }
#[test] fn test_imports_on_luau() { check_imports("luau"); }
#[test] fn test_imports_on_elixir() { check_imports("elixir"); }
#[test] fn test_imports_on_ocaml() { check_imports("ocaml"); }

// ---------------------------------------------------------------------------- loc
#[test] fn test_loc_on_python() { check_loc("python"); }
#[test] fn test_loc_on_typescript() { check_loc("typescript"); }
#[test] fn test_loc_on_javascript() { check_loc("javascript"); }
#[test] fn test_loc_on_go() { check_loc("go"); }
#[test] fn test_loc_on_rust() { check_loc("rust"); }
#[test] fn test_loc_on_java() { check_loc("java"); }
#[test] fn test_loc_on_c() { check_loc("c"); }
#[test] fn test_loc_on_cpp() { check_loc("cpp"); }
#[test] fn test_loc_on_ruby() { check_loc("ruby"); }
#[test] fn test_loc_on_kotlin() { check_loc("kotlin"); }
#[test] fn test_loc_on_swift() { check_loc("swift"); }
#[test] fn test_loc_on_csharp() { check_loc("csharp"); }
#[test] fn test_loc_on_scala() { check_loc("scala"); }
#[test] fn test_loc_on_php() { check_loc("php"); }
#[test] fn test_loc_on_lua() { check_loc("lua"); }
#[test] fn test_loc_on_luau() { check_loc("luau"); }
#[test] fn test_loc_on_elixir() { check_loc("elixir"); }
#[test] fn test_loc_on_ocaml() { check_loc("ocaml"); }

// ---------------------------------------------------------------------------- complexity
#[test] fn test_complexity_on_python() { check_complexity("python"); }
#[test] fn test_complexity_on_typescript() { check_complexity("typescript"); }
#[test] fn test_complexity_on_javascript() { check_complexity("javascript"); }
#[test] fn test_complexity_on_go() { check_complexity("go"); }
#[test] fn test_complexity_on_rust() { check_complexity("rust"); }
#[test] fn test_complexity_on_java() { check_complexity("java"); }
#[test] fn test_complexity_on_c() { check_complexity("c"); }
#[test] fn test_complexity_on_cpp() { check_complexity("cpp"); }
#[test] fn test_complexity_on_ruby() { check_complexity("ruby"); }
#[test] fn test_complexity_on_kotlin() { check_complexity("kotlin"); }
#[test] fn test_complexity_on_swift() { check_complexity("swift"); }
#[test] fn test_complexity_on_csharp() { check_complexity("csharp"); }
#[test] fn test_complexity_on_scala() { check_complexity("scala"); }
#[test] fn test_complexity_on_php() { check_complexity("php"); }
#[test] fn test_complexity_on_lua() { check_complexity("lua"); }
#[test] fn test_complexity_on_luau() { check_complexity("luau"); }
#[test] fn test_complexity_on_elixir() { check_complexity("elixir"); }
#[test] fn test_complexity_on_ocaml() { check_complexity("ocaml"); }

// ---------------------------------------------------------------------------- cognitive
#[test] fn test_cognitive_on_python() { check_cognitive("python"); }
#[test] fn test_cognitive_on_typescript() { check_cognitive("typescript"); }
#[test] fn test_cognitive_on_javascript() { check_cognitive("javascript"); }
#[test] fn test_cognitive_on_go() { check_cognitive("go"); }
#[test] fn test_cognitive_on_rust() { check_cognitive("rust"); }
#[test] fn test_cognitive_on_java() { check_cognitive("java"); }
#[test] fn test_cognitive_on_c() { check_cognitive("c"); }
#[test] fn test_cognitive_on_cpp() { check_cognitive("cpp"); }
#[test] fn test_cognitive_on_ruby() { check_cognitive("ruby"); }
#[test] fn test_cognitive_on_kotlin() { check_cognitive("kotlin"); }
#[test] fn test_cognitive_on_swift() { check_cognitive("swift"); }
#[test] fn test_cognitive_on_csharp() { check_cognitive("csharp"); }
#[test] fn test_cognitive_on_scala() { check_cognitive("scala"); }
#[test] fn test_cognitive_on_php() { check_cognitive("php"); }
#[test] fn test_cognitive_on_lua() { check_cognitive("lua"); }
#[test] fn test_cognitive_on_luau() { check_cognitive("luau"); }
#[test] fn test_cognitive_on_elixir() { check_cognitive("elixir"); }
#[test] fn test_cognitive_on_ocaml() { check_cognitive("ocaml"); }

// ---------------------------------------------------------------------------- halstead
#[test] fn test_halstead_on_python() { check_halstead("python"); }
#[test] fn test_halstead_on_typescript() { check_halstead("typescript"); }
#[test] fn test_halstead_on_javascript() { check_halstead("javascript"); }
#[test] fn test_halstead_on_go() { check_halstead("go"); }
#[test] fn test_halstead_on_rust() { check_halstead("rust"); }
#[test] fn test_halstead_on_java() { check_halstead("java"); }
#[test] fn test_halstead_on_c() { check_halstead("c"); }
#[test] fn test_halstead_on_cpp() { check_halstead("cpp"); }
#[test] fn test_halstead_on_ruby() { check_halstead("ruby"); }
#[test] fn test_halstead_on_kotlin() { check_halstead("kotlin"); }
#[test] fn test_halstead_on_swift() { check_halstead("swift"); }
#[test] fn test_halstead_on_csharp() { check_halstead("csharp"); }
#[test] fn test_halstead_on_scala() { check_halstead("scala"); }
#[test] fn test_halstead_on_php() { check_halstead("php"); }
#[test] fn test_halstead_on_lua() { check_halstead("lua"); }
#[test] fn test_halstead_on_luau() { check_halstead("luau"); }
#[test] fn test_halstead_on_elixir() { check_halstead("elixir"); }
#[test] fn test_halstead_on_ocaml() { check_halstead("ocaml"); }

// ---------------------------------------------------------------------------- smells
#[test] fn test_smells_on_python() { check_smells("python"); }
#[test] fn test_smells_on_typescript() { check_smells("typescript"); }
#[test] fn test_smells_on_javascript() { check_smells("javascript"); }
#[test] fn test_smells_on_go() { check_smells("go"); }
#[test] fn test_smells_on_rust() { check_smells("rust"); }
#[test] fn test_smells_on_java() { check_smells("java"); }
#[test] fn test_smells_on_c() { check_smells("c"); }
#[test] fn test_smells_on_cpp() { check_smells("cpp"); }
#[test] fn test_smells_on_ruby() { check_smells("ruby"); }
#[test] fn test_smells_on_kotlin() { check_smells("kotlin"); }
#[test] fn test_smells_on_swift() { check_smells("swift"); }
#[test] fn test_smells_on_csharp() { check_smells("csharp"); }
#[test] fn test_smells_on_scala() { check_smells("scala"); }
#[test] fn test_smells_on_php() { check_smells("php"); }
#[test] fn test_smells_on_lua() { check_smells("lua"); }
#[test] fn test_smells_on_luau() { check_smells("luau"); }
#[test] fn test_smells_on_elixir() { check_smells("elixir"); }
#[test] fn test_smells_on_ocaml() { check_smells("ocaml"); }

// ---------------------------------------------------------------------------- calls
#[test] fn test_calls_on_python() { check_calls("python"); }
#[test] fn test_calls_on_typescript() { check_calls("typescript"); }
#[test] fn test_calls_on_javascript() { check_calls("javascript"); }
#[test] fn test_calls_on_go() { check_calls("go"); }
#[test] fn test_calls_on_rust() { check_calls("rust"); }
#[test] fn test_calls_on_java() { check_calls("java"); }
#[test] fn test_calls_on_c() { check_calls("c"); }
#[test] fn test_calls_on_cpp() { check_calls("cpp"); }
#[test] fn test_calls_on_ruby() { check_calls("ruby"); }
#[test] fn test_calls_on_kotlin() { check_calls("kotlin"); }
#[test] fn test_calls_on_swift() { check_calls("swift"); }
#[test] fn test_calls_on_csharp() { check_calls("csharp"); }
#[test] fn test_calls_on_scala() { check_calls("scala"); }
#[test] fn test_calls_on_php() { check_calls("php"); }
#[test] fn test_calls_on_lua() { check_calls("lua"); }
#[test] fn test_calls_on_luau() { check_calls("luau"); }
#[test] fn test_calls_on_elixir() { check_calls("elixir"); }
#[test] fn test_calls_on_ocaml() { check_calls("ocaml"); }

// ---------------------------------------------------------------------------- dead
#[test] fn test_dead_on_python() { check_dead("python"); }
#[test] fn test_dead_on_typescript() { check_dead("typescript"); }
#[test] fn test_dead_on_javascript() { check_dead("javascript"); }
#[test] fn test_dead_on_go() { check_dead("go"); }
#[test] fn test_dead_on_rust() { check_dead("rust"); }
#[test] fn test_dead_on_java() { check_dead("java"); }
#[test] fn test_dead_on_c() { check_dead("c"); }
#[test] fn test_dead_on_cpp() { check_dead("cpp"); }
#[test] fn test_dead_on_ruby() { check_dead("ruby"); }
#[test] fn test_dead_on_kotlin() { check_dead("kotlin"); }
#[test] fn test_dead_on_swift() { check_dead("swift"); }
#[test] fn test_dead_on_csharp() { check_dead("csharp"); }
#[test] fn test_dead_on_scala() { check_dead("scala"); }
#[test] fn test_dead_on_php() { check_dead("php"); }
#[test] fn test_dead_on_lua() { check_dead("lua"); }
#[test] fn test_dead_on_luau() { check_dead("luau"); }
#[test] fn test_dead_on_elixir() { check_dead("elixir"); }
#[test] fn test_dead_on_ocaml() { check_dead("ocaml"); }

// ---------------------------------------------------------------------------- references
#[test] fn test_references_on_python() { check_references("python"); }
#[test] fn test_references_on_typescript() { check_references("typescript"); }
#[test] fn test_references_on_javascript() { check_references("javascript"); }
#[test] fn test_references_on_go() { check_references("go"); }
#[test] fn test_references_on_rust() { check_references("rust"); }
#[test] fn test_references_on_java() { check_references("java"); }
#[test] fn test_references_on_c() { check_references("c"); }
#[test] fn test_references_on_cpp() { check_references("cpp"); }
#[test] fn test_references_on_ruby() { check_references("ruby"); }
#[test] fn test_references_on_kotlin() { check_references("kotlin"); }
#[test] fn test_references_on_swift() { check_references("swift"); }
#[test] fn test_references_on_csharp() { check_references("csharp"); }
#[test] fn test_references_on_scala() { check_references("scala"); }
#[test] fn test_references_on_php() { check_references("php"); }
#[test] fn test_references_on_lua() { check_references("lua"); }
#[test] fn test_references_on_luau() { check_references("luau"); }
#[test] fn test_references_on_elixir() { check_references("elixir"); }
#[test] fn test_references_on_ocaml() { check_references("ocaml"); }

// ---------------------------------------------------------------------------- impact
#[test] fn test_impact_on_python() { check_impact("python"); }
#[test] fn test_impact_on_typescript() { check_impact("typescript"); }
#[test] fn test_impact_on_javascript() { check_impact("javascript"); }
#[test] fn test_impact_on_go() { check_impact("go"); }
#[test] fn test_impact_on_rust() { check_impact("rust"); }
#[test] fn test_impact_on_java() { check_impact("java"); }
#[test] fn test_impact_on_c() { check_impact("c"); }
#[test] fn test_impact_on_cpp() { check_impact("cpp"); }
#[test] fn test_impact_on_ruby() { check_impact("ruby"); }
#[test] fn test_impact_on_kotlin() { check_impact("kotlin"); }
#[test] fn test_impact_on_swift() { check_impact("swift"); }
#[test] fn test_impact_on_csharp() { check_impact("csharp"); }
#[test] fn test_impact_on_scala() { check_impact("scala"); }
#[test] fn test_impact_on_php() { check_impact("php"); }
#[test] fn test_impact_on_lua() { check_impact("lua"); }
#[test] fn test_impact_on_luau() { check_impact("luau"); }
#[test] fn test_impact_on_elixir() { check_impact("elixir"); }
#[test] fn test_impact_on_ocaml() { check_impact("ocaml"); }

// ---------------------------------------------------------------------------- patterns
#[test] fn test_patterns_on_python() { check_patterns("python"); }
#[test] fn test_patterns_on_typescript() { check_patterns("typescript"); }
#[test] fn test_patterns_on_javascript() { check_patterns("javascript"); }
#[test] fn test_patterns_on_go() { check_patterns("go"); }
#[test] fn test_patterns_on_rust() { check_patterns("rust"); }
#[test] fn test_patterns_on_java() { check_patterns("java"); }
#[test] fn test_patterns_on_c() { check_patterns("c"); }
#[test] fn test_patterns_on_cpp() { check_patterns("cpp"); }
#[test] fn test_patterns_on_ruby() { check_patterns("ruby"); }
#[test] fn test_patterns_on_kotlin() { check_patterns("kotlin"); }
#[test] fn test_patterns_on_swift() { check_patterns("swift"); }
#[test] fn test_patterns_on_csharp() { check_patterns("csharp"); }
#[test] fn test_patterns_on_scala() { check_patterns("scala"); }
#[test] fn test_patterns_on_php() { check_patterns("php"); }
#[test] fn test_patterns_on_lua() { check_patterns("lua"); }
#[test] fn test_patterns_on_luau() { check_patterns("luau"); }
#[test] fn test_patterns_on_elixir() { check_patterns("elixir"); }
#[test] fn test_patterns_on_ocaml() { check_patterns("ocaml"); }
