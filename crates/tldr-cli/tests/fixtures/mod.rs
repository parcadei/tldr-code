//! Canonical minimal fixtures for each of the 18 supported languages.
//!
//! Each fixture writes 2 source files + (where applicable) a manifest into
//! the provided `root` tempdir, with these invariants:
//!
//! 1. **3 functions total** across 2 files.
//! 2. **File A** (entry) defines `helper` (returns a constant) and `main`
//!    (calls `helper` and also calls `b_util` from file B).
//! 3. **File B** defines `b_util` (returns a constant) — imported by File A.
//! 4. Total **2 call edges**: `main -> helper` and `main -> b_util`.
//!
//! These invariants let the (command × language) matrix in
//! `language_command_matrix.rs` make strong semantic assertions:
//! * `structure` must extract >= 1 function from at least one file.
//! * `calls` must find >= 1 edge.
//! * `references helper` must find >= 1 call.
//! * `impact helper` must locate a target.
//!
//! Each language's fixture uses its canonical manifest (matching
//! `Language::from_directory`'s manifest precedence table in
//! `tldr-core/src/types.rs`) so that `tldr structure` autodetects the
//! correct language.
//!
//! See also: `language_autodetect_tests.rs` for VAL-008 autodetect coverage.

use std::fs;
use std::path::Path;

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
// Python
// ============================================================================

fn build_python(root: &Path) {
    write_file(&root.join("pyproject.toml"), "[project]\nname = \"x\"\n");
    write_file(
        &root.join("main.py"),
        "from util import b_util\n\n\
         def helper():\n    return 1\n\n\
         def main():\n    helper()\n    b_util()\n",
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
         export function main(): void {\n  helper();\n  b_util();\n}\n",
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
         export function main() {\n  helper();\n  b_util();\n}\n",
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
         func main() {\n  helper()\n  util.BUtil()\n}\n",
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
         fn main() {\n    let _ = helper();\n    let _ = util::b_util();\n}\n",
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
         int main(void) {\n    helper();\n    b_util();\n    return 0;\n}\n",
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
         int main() {\n    helper();\n    b_util();\n    return 0;\n}\n",
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
    // Use parenthesized calls (`helper()`, not bare `helper`) so the
    // tree-sitter-ruby grammar emits a `call` node — the Ruby handler's
    // `extract_calls_from_node` only walks `call` nodes.
    //
    // See `crates/tldr-core/src/callgraph/languages/ruby.rs:256` — the
    // iteration filters to `child.kind() == "call"`, and bareword method
    // dispatch `helper` (no parens) parses as an `identifier`, not a
    // `call` node, so edges would be missed.
    write_file(
        &root.join("main.rb"),
        "require_relative 'util'\n\n\
         def helper\n  1\nend\n\n\
         def main\n  helper()\n  b_util()\nend\n",
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
         fun main() {\n    helper()\n    bUtil()\n}\n",
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
         function main() {\n    helper();\n    b_util();\n}\n",
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
         function main()\n    helper()\n    util.b_util()\nend\n",
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
         local function main()\n    helper()\n    util.b_util()\nend\n",
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
         \x20 ()\n",
    );
    write_file(
        &root.join("util.ml"),
        "let b_util () = 2\n",
    );
}
