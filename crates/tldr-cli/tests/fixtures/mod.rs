//! Canonical minimal fixtures for each of the 18 supported languages.
//!
//! Each fixture writes 2 source files + (where applicable) a manifest into
//! the provided `root` tempdir, with these invariants:
//!
//! 1. **4 functions total** across 2 files.
//! 2. **File A** (entry) defines:
//!    - `helper` (returns a constant; called by `main`).
//!    - `main` (calls `helper` and `b_util`).
//!    - `dead_helper` (returns a constant; **never called from anywhere**
//!      — exists solely so `tldr dead` reports >= 1 unreachable function).
//! 3. **File B** defines `b_util` (returns a constant) — imported by File A.
//! 4. Total **2 call edges**: `main -> helper` and `main -> b_util`.
//! 5. **1 unreachable function**: `dead_helper`.
//!
//! These invariants let the (command × language) matrix in
//! `language_command_matrix.rs` and `exhaustive_matrix.rs` make strong
//! semantic assertions:
//! * `structure` must extract >= 1 function from at least one file.
//! * `calls` must find >= 2 edges (main->helper, main->b_util).
//! * `dead` must report >= 1 dead function (`dead_helper`).
//! * `references helper` must find >= 1 call.
//! * `impact helper` must locate a target.
//!
//! VAL-018: `dead_helper` was added to all 18 fixtures so that
//! `check_dead` can be tightened from "total_functions > 0" to
//! "total_dead >= 1", catching dead-code analyzers that silently emit an
//! empty `dead_functions` array. Existing matrix assertions about
//! function counts (where present) are tolerant of the increase
//! (`>= N` form), so the addition is backward-compatible.
//!
//! Each language's fixture uses its canonical manifest (matching
//! `Language::from_directory`'s manifest precedence table in
//! `tldr-core/src/types.rs`) so that `tldr structure` autodetects the
//! correct language.
//!
//! See also: `language_autodetect_tests.rs` for VAL-008 autodetect coverage.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Write a file, creating parent directories as needed.
pub fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create_dir_all");
    }
    fs::write(path, contents).expect("write file");
}

/// Build the canonical 3-function, 2-file fixture for the given language.
///
/// The `lang` string matches `Language::as_str()`: lowercase, no spaces
/// (e.g. `"python"`, `"typescript"`, `"cpp"`, `"csharp"`).
///
/// Panics on unknown language. Tests that call this ensure the 18 supported
/// languages are exhaustively covered.
pub fn build_fixture(lang: &str, root: &Path) {
    match lang {
        "python" => build_python(root),
        "typescript" => build_typescript(root),
        "javascript" => build_javascript(root),
        "go" => build_go(root),
        "rust" => build_rust(root),
        "java" => build_java(root),
        "c" => build_c(root),
        "cpp" => build_cpp(root),
        "ruby" => build_ruby(root),
        "kotlin" => build_kotlin(root),
        "swift" => build_swift(root),
        "csharp" => build_csharp(root),
        "scala" => build_scala(root),
        "php" => build_php(root),
        "lua" => build_lua(root),
        "luau" => build_luau(root),
        "elixir" => build_elixir(root),
        "ocaml" => build_ocaml(root),
        other => panic!("unknown language for fixture: {:?}", other),
    }
}

// ============================================================================
// Git-wrapped fixture (VAL-017)
// ============================================================================
//
// `tldr churn` and `tldr hotspots` consume `git log` output and require a
// real repository with commit history. The bare canonical fixture writes
// only files; this helper wraps it in `git init` + 3 commits so churn
// sees `commit_count >= 3` (default `min_commits` for hotspots, see
// `crates/tldr-core/src/quality/hotspots.rs:387`).

/// Path to the entry file (the one defining `helper` and `main`) within
/// a fixture root, by language. Mirrors `entry_file` in
/// `exhaustive_matrix.rs`. Kept duplicated here so `build_git_fixture`
/// can be self-contained at the fixtures layer.
fn fixture_entry_relpath(lang: &str) -> &'static str {
    match lang {
        "python" => "main.py",
        "typescript" => "main.ts",
        "javascript" => "main.js",
        "go" => "main.go",
        "rust" => "src/main.rs",
        "java" => "Main.java",
        "c" => "main.c",
        "cpp" => "main.cpp",
        "ruby" => "main.rb",
        "kotlin" => "Main.kt",
        "swift" => "Main.swift",
        "csharp" => "Program.cs",
        "scala" => "Main.scala",
        "php" => "main.php",
        "lua" => "main.lua",
        "luau" => "main.luau",
        "elixir" => "main.ex",
        "ocaml" => "main.ml",
        other => panic!("unknown language for fixture entry: {:?}", other),
    }
}

/// Run a git command with deterministic test environment, panicking
/// (with a clear message naming the command + stderr) if it fails.
///
/// All `user.*` config is set per-repo so we never touch the user's
/// global git config, and `commit.gpgsign=false` defends against any
/// global gpg.signing setting that would block `git commit` in CI/macOS.
fn run_git(cwd: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd);
    cmd.args(args);
    // Override $HOME-derived globals to avoid picking up the developer's
    // `~/.gitconfig` settings (e.g. signing keys). `GIT_CONFIG_GLOBAL=`
    // empty means "no global config".
    cmd.env("GIT_CONFIG_GLOBAL", "");
    cmd.env("GIT_CONFIG_SYSTEM", "");
    cmd.env("GIT_AUTHOR_NAME", "TldrTest");
    cmd.env("GIT_AUTHOR_EMAIL", "tldr-test@example.com");
    cmd.env("GIT_COMMITTER_NAME", "TldrTest");
    cmd.env("GIT_COMMITTER_EMAIL", "tldr-test@example.com");
    let output = cmd.output().unwrap_or_else(|e| {
        panic!(
            "failed to spawn `git {}`: {}\n(is git installed and on PATH?)",
            args.join(" "),
            e
        );
    });
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "git command failed in {:?}: `git {}`\nexit: {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            cwd,
            args.join(" "),
            output.status.code(),
            stdout,
            stderr
        );
    }
}

/// A language-appropriate single-line-comment line to append. The
/// trailing newline is included so subsequent appends create one new
/// line per commit (no merging onto the previous comment line).
///
/// Each marker is the canonical "rest-of-line is a comment" sigil for
/// that language so the appended text is syntactically valid and the
/// post-touch file still parses cleanly under
/// `tldr structure` / `tldr cognitive` / etc.
fn comment_line(lang: &str, body: &str) -> String {
    match lang {
        // C-family line comment.
        "typescript" | "javascript" | "go" | "rust" | "java" | "c"
        | "cpp" | "kotlin" | "swift" | "csharp" | "scala" | "php" => {
            format!("// {}\n", body)
        }
        // `#` comments.
        "python" | "ruby" => format!("# {}\n", body),
        // `--` comments.
        "lua" | "luau" => format!("-- {}\n", body),
        // `# ` comments (Elixir).
        "elixir" => format!("# {}\n", body),
        // OCaml block comments — single-line `(* ... *)` is the
        // canonical idiom for trailing comments in `.ml` files.
        "ocaml" => format!("(* {} *)\n", body),
        other => panic!("comment_line: unknown lang {:?}", other),
    }
}

/// Build the canonical 2-file 3-function fixture for `lang` in `root`,
/// then wrap it in a git repository with **3 commits** that touch the
/// entry file:
///
///   1. Initial commit ("initial") — entire fixture.
///   2. Append-line commit ("touch1") — appends a single
///      language-appropriate comment line to the entry file.
///   3. Append-line commit ("touch2") — appends another comment line.
///
/// 3 commits is the minimum that satisfies `tldr hotspots`'s default
/// `min_commits = 3` filter (see
/// `crates/tldr-core/src/quality/hotspots.rs:387`); fewer would cause
/// hotspots to filter the file out and emit an empty `hotspots` array.
///
/// The appended lines use language-appropriate comment syntax (see
/// `comment_line`) so the post-touch file still parses cleanly. This
/// matters for `tldr hotspots`, which calls `analyze_cognitive` on each
/// file in the report.
///
/// All git operations run with `GIT_CONFIG_GLOBAL=""` /
/// `GIT_CONFIG_SYSTEM=""` plus inline author/committer env vars, so the
/// helper is deterministic and never reads/writes the developer's
/// global `~/.gitconfig`.
///
/// The `--no-gpg-sign` flag on `commit` is redundant (gpg is already
/// disabled by the empty global config) but cheap and explicit.
pub fn build_git_fixture(lang: &str, root: &Path) {
    // Step 1: write the canonical fixture.
    build_fixture(lang, root);

    // Step 2: init repo + local-only identity config.
    run_git(root, &["init", "--quiet", "--initial-branch=main"]);
    run_git(root, &["config", "--local", "user.email", "tldr-test@example.com"]);
    run_git(root, &["config", "--local", "user.name", "TldrTest"]);
    run_git(root, &["config", "--local", "commit.gpgsign", "false"]);

    // Step 3: commit 1 (initial fixture).
    run_git(root, &["add", "-A"]);
    run_git(
        root,
        &["commit", "-m", "initial", "--no-gpg-sign", "--quiet"],
    );

    // Step 4: commit 2 (append a comment line to entry file).
    let entry = root.join(fixture_entry_relpath(lang));
    let line2 = comment_line(lang, "touch1");
    append_trailing_line(&entry, &line2);
    run_git(root, &["add", "-A"]);
    run_git(
        root,
        &["commit", "-m", "touch1", "--no-gpg-sign", "--quiet"],
    );

    // Step 5: commit 3 (another comment line).
    let line3 = comment_line(lang, "touch2");
    append_trailing_line(&entry, &line3);
    run_git(root, &["add", "-A"]);
    run_git(
        root,
        &["commit", "-m", "touch2", "--no-gpg-sign", "--quiet"],
    );
}

/// Append a literal string to a file. Used by `build_git_fixture` to
/// create per-commit deltas.
fn append_trailing_line(path: &Path, line: &str) {
    let mut existing = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read entry file {:?} for append: {}", path, e));
    if !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(line);
    fs::write(path, existing).unwrap_or_else(|e| panic!("write entry file {:?}: {}", path, e));
}

// ============================================================================
// Python
// ============================================================================

fn build_python(root: &Path) {
    write_file(&root.join("pyproject.toml"), "[project]\nname = \"x\"\n");
    write_file(
        &root.join("main.py"),
        "from util import b_util\n\n\
         def helper():\n    return 1\n\n\
         def main():\n    helper()\n    b_util()\n\n\
         def dead_helper():\n    return 99\n",
    );
    write_file(&root.join("util.py"), "def b_util():\n    return 2\n");
}

// ============================================================================
// TypeScript
// ============================================================================

fn build_typescript(root: &Path) {
    write_file(&root.join("tsconfig.json"), "{}\n");
    write_file(
        &root.join("main.ts"),
        "import { b_util } from './util';\n\n\
         export function helper(): number { return 1; }\n\n\
         export function main(): void {\n  helper();\n  b_util();\n}\n\n\
         export function dead_helper(): number { return 99; }\n",
    );
    write_file(
        &root.join("util.ts"),
        "export function b_util(): number { return 2; }\n",
    );
}

// ============================================================================
// JavaScript
// ============================================================================

fn build_javascript(root: &Path) {
    write_file(
        &root.join("package.json"),
        "{\"name\":\"x\",\"version\":\"0.1.0\"}\n",
    );
    write_file(
        &root.join("main.js"),
        "import { b_util } from './util.js';\n\n\
         export function helper() { return 1; }\n\n\
         export function main() {\n  helper();\n  b_util();\n}\n\n\
         export function dead_helper() { return 99; }\n",
    );
    write_file(
        &root.join("util.js"),
        "export function b_util() { return 2; }\n",
    );
}

// ============================================================================
// Go
// ============================================================================

fn build_go(root: &Path) {
    write_file(
        &root.join("go.mod"),
        "module example.com/x\n\ngo 1.21\n",
    );
    write_file(
        &root.join("main.go"),
        "package main\n\nimport \"example.com/x/util\"\n\n\
         func helper() int { return 1 }\n\n\
         func main() {\n  helper()\n  util.BUtil()\n}\n\n\
         func dead_helper() int { return 99 }\n",
    );
    write_file(
        &root.join("util/util.go"),
        "package util\n\nfunc BUtil() int { return 2 }\n",
    );
}

// ============================================================================
// Rust
// ============================================================================

fn build_rust(root: &Path) {
    write_file(
        &root.join("Cargo.toml"),
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write_file(
        &root.join("src/main.rs"),
        "mod util;\n\n\
         fn helper() -> i32 { 1 }\n\n\
         fn main() {\n    let _ = helper();\n    let _ = util::b_util();\n}\n\n\
         fn dead_helper() -> i32 { 99 }\n",
    );
    write_file(
        &root.join("src/util.rs"),
        "pub fn b_util() -> i32 { 2 }\n",
    );
}

// ============================================================================
// Java
// ============================================================================

fn build_java(root: &Path) {
    write_file(&root.join("pom.xml"), "<project/>\n");
    // Java canonical convention: public class in file matching class name.
    write_file(
        &root.join("Main.java"),
        "class Main {\n\
         \x20   public static int helper() { return 1; }\n\
         \x20   public static void main(String[] args) {\n\
         \x20       helper();\n\
         \x20       Util.bUtil();\n\
         \x20   }\n\
         \x20   public static int deadHelper() { return 99; }\n\
         }\n",
    );
    write_file(
        &root.join("Util.java"),
        "class Util {\n\
         \x20   public static int bUtil() { return 2; }\n\
         }\n",
    );
}

// ============================================================================
// C
// ============================================================================

fn build_c(root: &Path) {
    write_file(
        &root.join("CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.10)\nproject(x C)\n",
    );
    write_file(
        &root.join("main.c"),
        "#include \"util.h\"\n\n\
         int helper(void) { return 1; }\n\n\
         int main(void) {\n    helper();\n    b_util();\n    return 0;\n}\n\n\
         int dead_helper(void) { return 99; }\n",
    );
    write_file(&root.join("util.h"), "int b_util(void);\n");
    write_file(
        &root.join("util.c"),
        "int b_util(void) { return 2; }\n",
    );
}

// ============================================================================
// C++
// ============================================================================

fn build_cpp(root: &Path) {
    write_file(
        &root.join("CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.10)\nproject(x CXX)\n",
    );
    write_file(
        &root.join("main.cpp"),
        "#include \"util.hpp\"\n\n\
         int helper() { return 1; }\n\n\
         int main() {\n    helper();\n    b_util();\n    return 0;\n}\n\n\
         int dead_helper() { return 99; }\n",
    );
    write_file(&root.join("util.hpp"), "int b_util();\n");
    write_file(
        &root.join("util.cpp"),
        "int b_util() { return 2; }\n",
    );
}

// ============================================================================
// Ruby
// ============================================================================

fn build_ruby(root: &Path) {
    write_file(
        &root.join("Gemfile"),
        "source 'https://rubygems.org'\n",
    );
    // Idiomatic bareword Ruby method dispatch: `helper`/`b_util` (no parens).
    // VAL-012 ensures the Ruby callgraph handler resolves these
    // identifiers to method calls (not local variable reads).
    write_file(
        &root.join("main.rb"),
        "require_relative 'util'\n\n\
         def helper\n  1\nend\n\n\
         def main\n  helper\n  b_util\nend\n\n\
         def dead_helper\n  99\nend\n",
    );
    write_file(
        &root.join("util.rb"),
        "def b_util\n  2\nend\n",
    );
}

// ============================================================================
// Kotlin
// ============================================================================

fn build_kotlin(root: &Path) {
    write_file(&root.join("build.gradle.kts"), "");
    // Make Kotlin outnumber Java so Gradle tie-break picks Kotlin.
    write_file(
        &root.join("Main.kt"),
        "fun helper(): Int = 1\n\n\
         fun main() {\n    helper()\n    bUtil()\n}\n\n\
         fun deadHelper(): Int = 99\n",
    );
    write_file(
        &root.join("Util.kt"),
        "fun bUtil(): Int = 2\n",
    );
}

// ============================================================================
// Swift
// ============================================================================

fn build_swift(root: &Path) {
    write_file(
        &root.join("Package.swift"),
        "// swift-tools-version:5.5\n",
    );
    write_file(
        &root.join("Main.swift"),
        "func helper() -> Int { return 1 }\n\n\
         func main() {\n    _ = helper()\n    _ = bUtil()\n}\n\n\
         func deadHelper() -> Int { return 99 }\n\n\
         main()\n",
    );
    write_file(
        &root.join("Util.swift"),
        "func bUtil() -> Int { return 2 }\n",
    );
}

// ============================================================================
// C#
// ============================================================================

fn build_csharp(root: &Path) {
    write_file(
        &root.join("App.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\"/>\n",
    );
    // Use lowercase `helper`/`b_util` method names to match the matrix
    // invariant — C# allows them, just non-idiomatic. Entry point `Main`
    // stays PascalCase (required by .NET convention but immaterial here).
    write_file(
        &root.join("Program.cs"),
        "class Program {\n\
         \x20   static int helper() { return 1; }\n\
         \x20   static void Main(string[] args) {\n\
         \x20       helper();\n\
         \x20       Util.b_util();\n\
         \x20   }\n\
         \x20   static int dead_helper() { return 99; }\n\
         }\n",
    );
    write_file(
        &root.join("Util.cs"),
        "class Util {\n\
         \x20   public static int b_util() { return 2; }\n\
         }\n",
    );
}

// ============================================================================
// Scala
// ============================================================================

fn build_scala(root: &Path) {
    write_file(
        &root.join("build.sbt"),
        "name := \"x\"\nscalaVersion := \"3.3.0\"\n",
    );
    write_file(
        &root.join("Main.scala"),
        "object Main {\n\
         \x20 def helper(): Int = 1\n\
         \x20 def main(args: Array[String]): Unit = {\n\
         \x20   helper()\n\
         \x20   Util.bUtil()\n\
         \x20 }\n\
         \x20 def deadHelper(): Int = 99\n\
         }\n",
    );
    write_file(
        &root.join("Util.scala"),
        "object Util {\n\
         \x20 def bUtil(): Int = 2\n\
         }\n",
    );
}

// ============================================================================
// PHP
// ============================================================================

fn build_php(root: &Path) {
    write_file(&root.join("composer.json"), "{\"name\":\"x/y\"}\n");
    write_file(
        &root.join("main.php"),
        "<?php\nrequire_once 'util.php';\n\n\
         function helper() { return 1; }\n\n\
         function main() {\n    helper();\n    b_util();\n}\n\n\
         function dead_helper() { return 99; }\n",
    );
    write_file(
        &root.join("util.php"),
        "<?php\nfunction b_util() { return 2; }\n",
    );
}

// ============================================================================
// Lua
// ============================================================================

fn build_lua(root: &Path) {
    write_file(
        &root.join("x-1.0-1.rockspec"),
        "package = \"x\"\nversion = \"1.0-1\"\n",
    );
    write_file(
        &root.join("main.lua"),
        "local util = require('util')\n\n\
         function helper()\n    return 1\nend\n\n\
         function main()\n    helper()\n    util.b_util()\nend\n\n\
         function dead_helper()\n    return 99\nend\n",
    );
    write_file(
        &root.join("util.lua"),
        "local M = {}\n\
         function M.b_util()\n    return 2\nend\n\
         return M\n",
    );
}

// ============================================================================
// Luau
// ============================================================================

fn build_luau(root: &Path) {
    write_file(
        &root.join("default.project.json"),
        "{\"name\":\"x\",\"tree\":{\"$className\":\"DataModel\"}}\n",
    );
    write_file(
        &root.join("main.luau"),
        "local util = require('./util')\n\n\
         local function helper(): number\n    return 1\nend\n\n\
         local function main()\n    helper()\n    util.b_util()\nend\n\n\
         local function dead_helper(): number\n    return 99\nend\n",
    );
    write_file(
        &root.join("util.luau"),
        "local M = {}\n\
         function M.b_util(): number\n    return 2\nend\n\
         return M\n",
    );
}

// ============================================================================
// Elixir
// ============================================================================

fn build_elixir(root: &Path) {
    write_file(
        &root.join("mix.exs"),
        "defmodule X.MixProject do\nend\n",
    );
    write_file(
        &root.join("main.ex"),
        "defmodule Main do\n\
         \x20 def helper, do: 1\n\
         \x20 def main do\n\
         \x20   helper()\n\
         \x20   Util.b_util()\n\
         \x20 end\n\
         \x20 def dead_helper, do: 99\n\
         end\n",
    );
    write_file(
        &root.join("util.ex"),
        "defmodule Util do\n\
         \x20 def b_util, do: 2\n\
         end\n",
    );
}

// ============================================================================
// OCaml
// ============================================================================

fn build_ocaml(root: &Path) {
    write_file(&root.join("dune-project"), "(lang dune 3.0)\n");
    write_file(
        &root.join("main.ml"),
        "let helper () = 1\n\n\
         let main () =\n\
         \x20 let _ = helper () in\n\
         \x20 let _ = Util.b_util () in\n\
         \x20 ()\n\n\
         let dead_helper () = 99\n",
    );
    write_file(
        &root.join("util.ml"),
        "let b_util () = 2\n",
    );
}
