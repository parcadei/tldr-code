//! VAL-008: End-to-end language autodetect tests for `tldr structure`.
//!
//! Each test creates a canonical project fixture (a manifest file, if the
//! language has one, plus a minimal source file, plus Python bait to rule
//! out extension-majority noise) and runs `tldr structure <tempdir> --format
//! json --quiet`. The `language` field of the JSON output must match the
//! expected language name.
//!
//! These tests prove the *CLI pipeline* autodetects correctly end-to-end,
//! not just the `Language::from_directory` unit — they exercise argument
//! parsing, the `StructureArgs::run` default-lang logic, the code-structure
//! extractor, and the JSON serializer together.
//!
//! See also: `crates/tldr-core/src/types.rs` `test_from_directory_detects_*`
//! for the detector-level unit tests.

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Get the path to the tldr binary under test.
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

/// Write a file, creating parent directories if needed.
fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create_dir_all");
    }
    fs::write(path, contents).expect("write file");
}

/// Sprinkle `count` empty Python files into `dir` to bait extension majority.
/// Any passing test thus proves the **manifest** (not extension-count) is
/// responsible for the detected language.
fn sprinkle_python_bait(dir: &Path, count: usize) {
    for i in 0..count {
        fs::write(dir.join(format!("bait_{}.py", i)), "").unwrap();
    }
}

/// Invoke `tldr structure <dir>` and return the parsed JSON output.
fn run_structure_and_parse_json(dir: &Path) -> serde_json::Value {
    let mut cmd = tldr_cmd();
    cmd.arg("structure")
        .arg(dir)
        .arg("--format")
        .arg("json")
        .arg("--quiet");
    let output = cmd.output().expect("spawn tldr");
    assert!(
        output.status.success(),
        "tldr structure failed on {}: status={:?}\nstdout={}\nstderr={}",
        dir.display(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "could not parse tldr structure JSON output: {}\nstdout={}",
            e,
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// Assert that `tldr structure` autodetects the expected language on the
/// fixture directory. Fails with a helpful message on mismatch.
fn assert_detected(dir: &Path, expected: &str) {
    let v = run_structure_and_parse_json(dir);
    let actual = v.get("language").and_then(|s| s.as_str()).unwrap_or("");
    assert_eq!(
        actual,
        expected,
        "tldr structure reported language={:?}, expected {:?}. Full JSON: {}",
        actual,
        expected,
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| "<unprintable>".into())
    );
}

// ============================================================================
// 11 pre-VAL-008 languages
// ============================================================================

#[test]
fn test_tldr_structure_autodetects_python() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("pyproject.toml"),
        "[project]\nname=\"x\"\n",
    );
    write_file(&tmp.path().join("main.py"), "def x(): pass\n");
    assert_detected(tmp.path(), "python");
}

#[test]
fn test_tldr_structure_autodetects_typescript() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp.path().join("tsconfig.json"), "{}");
    write_file(
        &tmp.path().join("index.ts"),
        "export const x: number = 1;\n",
    );
    sprinkle_python_bait(tmp.path(), 3);
    assert_detected(tmp.path(), "typescript");
}

#[test]
fn test_tldr_structure_autodetects_javascript() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("package.json"),
        r#"{"name":"x","dependencies":{"express":"1.0.0"}}"#,
    );
    write_file(&tmp.path().join("index.js"), "export const x = 1;\n");
    assert_detected(tmp.path(), "javascript");
}

#[test]
fn test_tldr_structure_autodetects_go() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("go.mod"),
        "module example.com/x\ngo 1.21\n",
    );
    write_file(
        &tmp.path().join("main.go"),
        "package main\nfunc main() {}\n",
    );
    sprinkle_python_bait(tmp.path(), 3);
    assert_detected(tmp.path(), "go");
}

#[test]
fn test_tldr_structure_autodetects_rust() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("Cargo.toml"),
        "[package]\nname = \"x\"\nversion = \"0.1\"\nedition = \"2021\"\n",
    );
    write_file(&tmp.path().join("src/main.rs"), "pub fn x() {}\n");
    assert_detected(tmp.path(), "rust");
}

#[test]
fn test_tldr_structure_autodetects_java() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp.path().join("pom.xml"), "<project/>");
    write_file(
        &tmp.path().join("App.java"),
        "class App { public static void main(String[] a){} }\n",
    );
    assert_detected(tmp.path(), "java");
}

#[test]
fn test_tldr_structure_autodetects_kotlin() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp.path().join("build.gradle.kts"), "");
    // Make Kotlin outnumber Java so Gradle tie-break picks Kotlin.
    write_file(&tmp.path().join("App.kt"), "fun main() {}\n");
    write_file(&tmp.path().join("Util.kt"), "fun util() {}\n");
    assert_detected(tmp.path(), "kotlin");
}

#[test]
fn test_tldr_structure_autodetects_ruby() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("Gemfile"),
        "source 'https://rubygems.org'\n",
    );
    write_file(&tmp.path().join("app.rb"), "def x; end\n");
    assert_detected(tmp.path(), "ruby");
}

#[test]
fn test_tldr_structure_autodetects_php() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp.path().join("composer.json"), r#"{"name":"x/y"}"#);
    write_file(&tmp.path().join("index.php"), "<?php function x(){}\n");
    assert_detected(tmp.path(), "php");
}

#[test]
fn test_tldr_structure_autodetects_elixir() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("mix.exs"),
        "defmodule X.MixProject do\nend\n",
    );
    write_file(
        &tmp.path().join("lib.ex"),
        "defmodule X do\ndef y(), do: :ok\nend\n",
    );
    assert_detected(tmp.path(), "elixir");
}

#[test]
fn test_tldr_structure_autodetects_swift() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("Package.swift"),
        "// swift-tools-version:5.5\n",
    );
    write_file(&tmp.path().join("App.swift"), "func x(){}\n");
    assert_detected(tmp.path(), "swift");
}

// ============================================================================
// 7 VAL-008 newly-covered languages
// ============================================================================

#[test]
fn test_tldr_structure_autodetects_c() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.10)\nproject(x C)\n",
    );
    write_file(&tmp.path().join("main.c"), "int main(){return 0;}\n");
    sprinkle_python_bait(tmp.path(), 3);
    assert_detected(tmp.path(), "c");
}

#[test]
fn test_tldr_structure_autodetects_cpp() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.10)\nproject(x CXX)\n",
    );
    write_file(&tmp.path().join("main.cpp"), "int main(){return 0;}\n");
    write_file(&tmp.path().join("util.cpp"), "int util(){return 0;}\n");
    sprinkle_python_bait(tmp.path(), 3);
    assert_detected(tmp.path(), "cpp");
}

#[test]
fn test_tldr_structure_autodetects_csharp() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("App.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\"/>",
    );
    write_file(
        &tmp.path().join("Program.cs"),
        "class X { static void Main(){} }\n",
    );
    sprinkle_python_bait(tmp.path(), 3);
    assert_detected(tmp.path(), "csharp");
}

#[test]
fn test_tldr_structure_autodetects_scala() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("build.sbt"),
        "name := \"x\"\nscalaVersion := \"3.3.0\"\n",
    );
    write_file(
        &tmp.path().join("Main.scala"),
        "object X { def main(args: Array[String]): Unit = {} }\n",
    );
    sprinkle_python_bait(tmp.path(), 3);
    assert_detected(tmp.path(), "scala");
}

#[test]
fn test_tldr_structure_autodetects_lua() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("x-1.0-1.rockspec"),
        "package = \"x\"\nversion = \"1.0-1\"\n",
    );
    write_file(&tmp.path().join("init.lua"), "function x() end\n");
    sprinkle_python_bait(tmp.path(), 3);
    assert_detected(tmp.path(), "lua");
}

#[test]
fn test_tldr_structure_autodetects_luau() {
    let tmp = TempDir::new().unwrap();
    write_file(
        &tmp.path().join("default.project.json"),
        r#"{"name":"x","tree":{"$className":"DataModel"}}"#,
    );
    write_file(&tmp.path().join("init.luau"), "function x() end\n");
    sprinkle_python_bait(tmp.path(), 3);
    assert_detected(tmp.path(), "luau");
}

#[test]
fn test_tldr_structure_autodetects_ocaml() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp.path().join("dune-project"), "(lang dune 3.0)\n");
    write_file(&tmp.path().join("lib.ml"), "let x () = ()\n");
    sprinkle_python_bait(tmp.path(), 3);
    assert_detected(tmp.path(), "ocaml");
}
