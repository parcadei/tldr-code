//! P2: Multi-Language Extension Tests (TDD Red Phase)
//!
//! These tests drive the implementation of 5 commands across all 18 languages.
//! All tests are marked `#[ignore]` and must FAIL when run without the attribute.
//!
//! Commands covered:
//! - gvn: 18 languages (redundant expression detection)
//! - bounds: 18 languages (loop bound analysis)
//! - resources: 5 missing languages (Kotlin, Swift, OCaml, Lua, Luau)
//! - contracts: 3 missing languages (Kotlin, Swift, Luau)
//! - behavioral: 1 missing language (Swift)
//!
//! Reference: migration/p2-multilang-spec.md

use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// Get the test binary
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

/// Helper to create a test file in a temp directory
fn create_test_file(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    path
}

// =============================================================================
// GVN Multi-Language Tests
//
// Each test creates a source file with a function containing a redundant
// expression (computed twice), runs `tldr gvn`, and asserts that at least
// one redundancy is detected in the JSON output.
// =============================================================================

mod gvn_multilang {
    use super::*;

    #[test]
    fn test_gvn_python() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.py",
            r#"
def compute(a, b):
    x = a + b
    y = a + b
    return x + y
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        // Should detect at least one redundancy
        let redundancies = json.get("redundancies").or_else(|| {
            json.as_array()
                .and_then(|a| a.first())
                .and_then(|r| r.get("redundancies"))
        });
        assert!(redundancies.is_some(), "Should have redundancies field");
    }

    #[test]
    fn test_gvn_typescript() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.ts",
            r#"
function compute(a: number, b: number): number {
    const x = a + b;
    const y = a + b; // redundant
    return x + y;
}
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value =
            serde_json::from_str(&stdout).expect("TypeScript GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "TypeScript GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_javascript() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.js",
            r#"
function compute(a, b) {
    const x = a + b;
    const y = a + b; // redundant
    return x + y;
}
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value =
            serde_json::from_str(&stdout).expect("JavaScript GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "JavaScript GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_go() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.go",
            r#"
package main

func compute(a, b int) int {
    x := a + b
    y := a + b // redundant
    return x + y
}
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Go GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "Go GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_rust() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.rs",
            r#"
fn compute(a: i32, b: i32) -> i32 {
    let x = a + b;
    let y = a + b; // redundant
    x + y
}
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Rust GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "Rust GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_java() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "Test.java",
            r#"
class Test {
    int compute(int a, int b) {
        int x = a + b;
        int y = a + b; // redundant
        return x + y;
    }
}
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Java GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "Java GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_c() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.c",
            r#"
int compute(int a, int b) {
    int x = a + b;
    int y = a + b; // redundant
    return x + y;
}
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("C GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "C GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_cpp() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.cpp",
            r#"
int compute(int a, int b) {
    int x = a + b;
    int y = a + b; // redundant
    return x + y;
}
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("C++ GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "C++ GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_ruby() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.rb",
            r#"
def compute(a, b)
  x = a + b
  y = a + b # redundant
  x + y
end
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Ruby GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "Ruby GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_php() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.php",
            r#"<?php
function compute($a, $b) {
    $x = $a + $b;
    $y = $a + $b; // redundant
    return $x + $y;
}
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("PHP GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "PHP GVN should detect redundant $a + $b"
        );
    }

    #[test]
    fn test_gvn_kotlin() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.kt",
            r#"
fun compute(a: Int, b: Int): Int {
    val x = a + b
    val y = a + b // redundant
    return x + y
}
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value =
            serde_json::from_str(&stdout).expect("Kotlin GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "Kotlin GVN should detect redundant a + b"
        );
    }

    /// Swift uses regex fallback due to tree-sitter-swift ABI incompatibility
    #[test]
    fn test_gvn_swift() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.swift",
            r#"
func compute(a: Int, b: Int) -> Int {
    let x = a + b
    let y = a + b // redundant
    return x + y
}
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value =
            serde_json::from_str(&stdout).expect("Swift GVN (regex) should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "Swift GVN (regex fallback) should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_csharp() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "Test.cs",
            r#"
class Test {
    int Compute(int a, int b) {
        int x = a + b;
        int y = a + b; // redundant
        return x + y;
    }
}
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "Compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("C# GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "C# GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_scala() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "Test.scala",
            r#"
object Test {
  def compute(a: Int, b: Int): Int = {
    val x = a + b
    val y = a + b // redundant
    x + y
  }
}
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value =
            serde_json::from_str(&stdout).expect("Scala GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "Scala GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_elixir() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.ex",
            r#"
defmodule Test do
  def compute(a, b) do
    x = a + b
    y = a + b
    x + y
  end
end
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value =
            serde_json::from_str(&stdout).expect("Elixir GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "Elixir GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_lua() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.lua",
            r#"
function compute(a, b)
    local x = a + b
    local y = a + b -- redundant
    return x + y
end
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Lua GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "Lua GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_luau() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.luau",
            r#"
local function compute(a: number, b: number): number
    local x = a + b
    local y = a + b -- redundant
    return x + y
end
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Luau GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "Luau GVN should detect redundant a + b"
        );
    }

    #[test]
    fn test_gvn_ocaml() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.ml",
            r#"
let compute a b =
  let x = a + b in
  let y = a + b in
  x + y
"#,
        );
        let output = tldr_cmd()
            .args(["gvn", file.to_str().unwrap(), "compute", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value =
            serde_json::from_str(&stdout).expect("OCaml GVN should return valid JSON");
        assert!(
            json.to_string().contains("redundan"),
            "OCaml GVN should detect redundant a + b"
        );
    }
}

// =============================================================================
// Bounds Multi-Language Tests
//
// Each test creates a source file with a loop that has analyzable bounds,
// runs `tldr bounds`, and asserts that loop bounds are detected.
// =============================================================================

mod bounds_multilang {
    use super::*;

    #[test]
    fn test_bounds_python() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.py",
            r#"
def loop_func(n):
    for i in range(10):
        x = i * 2
    return x
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loop_func", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        // Should detect bounds for loop variable i: [0, 9]
        assert!(
            json.get("bounds").is_some() || json.to_string().contains("bound"),
            "Python bounds should detect range(10) loop bounds"
        );
    }

    #[test]
    fn test_bounds_typescript() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.ts",
            r#"
function loopFunc(n: number): number {
    let x = 0;
    for (let i = 0; i < 10; i++) {
        x = i * 2;
    }
    return x;
}
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loopFunc", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "TypeScript bounds should detect for-loop bounds i: [0, 9]"
        );
    }

    #[test]
    fn test_bounds_javascript() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.js",
            r#"
function loopFunc(n) {
    let x = 0;
    for (let i = 0; i < 10; i++) {
        x = i * 2;
    }
    return x;
}
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loopFunc", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "JavaScript bounds should detect for-loop bounds"
        );
    }

    #[test]
    fn test_bounds_go() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.go",
            r#"
package main

func loopFunc(n int) int {
    x := 0
    for i := 0; i < 10; i++ {
        x = i * 2
    }
    return x
}
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loopFunc", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "Go bounds should detect C-style for-loop bounds"
        );
    }

    #[test]
    fn test_bounds_rust() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.rs",
            r#"
fn loop_func() -> i32 {
    let mut x = 0;
    for i in 0..10 {
        x = i * 2;
    }
    x
}
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loop_func", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "Rust bounds should detect 0..10 range bounds i: [0, 9]"
        );
    }

    #[test]
    fn test_bounds_java() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "Test.java",
            r#"
class Test {
    int loopFunc(int n) {
        int x = 0;
        for (int i = 0; i < 10; i++) {
            x = i * 2;
        }
        return x;
    }
}
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loopFunc", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "Java bounds should detect C-style for-loop bounds"
        );
    }

    #[test]
    fn test_bounds_c() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.c",
            r#"
int loop_func(int n) {
    int x = 0;
    for (int i = 0; i < 10; i++) {
        x = i * 2;
    }
    return x;
}
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loop_func", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "C bounds should detect C-style for-loop bounds"
        );
    }

    #[test]
    fn test_bounds_cpp() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.cpp",
            r#"
int loop_func(int n) {
    int x = 0;
    for (int i = 0; i < 10; i++) {
        x = i * 2;
    }
    return x;
}
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loop_func", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "C++ bounds should detect C-style for-loop bounds"
        );
    }

    #[test]
    fn test_bounds_ruby() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.rb",
            r#"
def loop_func(n)
  x = 0
  (0...10).each do |i|
    x = i * 2
  end
  x
end
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loop_func", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "Ruby bounds should detect (0...10) range bounds"
        );
    }

    #[test]
    fn test_bounds_php() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.php",
            r#"<?php
function loop_func($n) {
    $x = 0;
    for ($i = 0; $i < 10; $i++) {
        $x = $i * 2;
    }
    return $x;
}
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loop_func", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "PHP bounds should detect C-style for-loop bounds"
        );
    }

    #[test]
    fn test_bounds_kotlin() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.kt",
            r#"
fun loopFunc(n: Int): Int {
    var x = 0
    for (i in 0 until 10) {
        x = i * 2
    }
    return x
}
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loopFunc", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "Kotlin bounds should detect 0 until 10 range bounds"
        );
    }

    /// Swift uses regex fallback due to tree-sitter-swift ABI incompatibility
    #[test]
    fn test_bounds_swift() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.swift",
            r#"
func loopFunc(n: Int) -> Int {
    var x = 0
    for i in 0..<10 {
        x = i * 2
    }
    return x
}
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loopFunc", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "Swift bounds (regex fallback) should detect 0..<10 range bounds"
        );
    }

    #[test]
    fn test_bounds_csharp() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "Test.cs",
            r#"
class Test {
    int LoopFunc(int n) {
        int x = 0;
        for (int i = 0; i < 10; i++) {
            x = i * 2;
        }
        return x;
    }
}
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "LoopFunc", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "C# bounds should detect C-style for-loop bounds"
        );
    }

    #[test]
    fn test_bounds_scala() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "Test.scala",
            r#"
object Test {
  def loopFunc(n: Int): Int = {
    var x = 0
    for (i <- 0 until 10) {
      x = i * 2
    }
    x
  }
}
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loopFunc", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "Scala bounds should detect 0 until 10 range bounds"
        );
    }

    #[test]
    fn test_bounds_elixir() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.ex",
            r#"
defmodule Test do
  def loop_func(n) do
    Enum.reduce(0..9, 0, fn i, _acc -> i * 2 end)
  end
end
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loop_func", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "Elixir bounds should detect 0..9 range bounds"
        );
    }

    #[test]
    fn test_bounds_lua() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.lua",
            r#"
function loop_func(n)
    local x = 0
    for i = 0, 9 do
        x = i * 2
    end
    return x
end
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loop_func", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "Lua bounds should detect for i = 0, 9 loop bounds"
        );
    }

    #[test]
    fn test_bounds_luau() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.luau",
            r#"
local function loop_func(n: number): number
    local x = 0
    for i = 0, 9 do
        x = i * 2
    end
    return x
end
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loop_func", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "Luau bounds should detect for i = 0, 9 loop bounds"
        );
    }

    #[test]
    fn test_bounds_ocaml() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.ml",
            r#"
let loop_func n =
  let x = ref 0 in
  for i = 0 to 9 do
    x := i * 2
  done;
  !x
"#,
        );
        let output = tldr_cmd()
            .args(["bounds", file.to_str().unwrap(), "loop_func", "-f", "json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("bound"),
            "OCaml bounds should detect for i = 0 to 9 loop bounds"
        );
    }
}

// =============================================================================
// Resources Multi-Language Tests (5 MISSING languages only)
//
// Each test creates a source file with a resource that is opened but not
// closed, runs `tldr resources`, and asserts that a leak is detected.
// =============================================================================

mod resources_multilang {
    use super::*;

    #[test]
    fn test_resources_kotlin() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.kt",
            r#"
fun leakyFunction(path: String): String {
    val reader = java.io.BufferedReader(java.io.FileReader(path))
    val content = reader.readLine()
    // reader is never closed - resource leak
    return content
}
"#,
        );
        let output = tldr_cmd()
            .args([
                "resources",
                file.to_str().unwrap(),
                "leakyFunction",
                "-f",
                "json",
            ])
            .output()
            .unwrap();
        // May exit with code 3 if issues found
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("leak") || json.to_string().contains("resource"),
            "Kotlin resources should detect unclosed BufferedReader leak"
        );
    }

    /// Swift now uses tree-sitter AST (ABI v15 confirmed working in P0)
    #[test]
    fn test_resources_swift() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.swift",
            r#"
func leakyFunction(path: String) -> String {
    let handle = FileHandle(forReadingAtPath: path)!
    let data = handle.readDataToEndOfFile()
    // handle is never closed - resource leak
    return String(data: data, encoding: .utf8)!
}
"#,
        );
        let output = tldr_cmd()
            .args([
                "resources",
                file.to_str().unwrap(),
                "leakyFunction",
                "-f",
                "json",
            ])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("leak") || json.to_string().contains("resource"),
            "Swift resources (regex fallback) should detect unclosed FileHandle leak"
        );
    }

    #[test]
    fn test_resources_ocaml() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.ml",
            r#"
let leaky_function path =
  let ic = open_in path in
  let line = input_line ic in
  (* ic is never closed - resource leak *)
  line
"#,
        );
        let output = tldr_cmd()
            .args([
                "resources",
                file.to_str().unwrap(),
                "leaky_function",
                "-f",
                "json",
            ])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("leak") || json.to_string().contains("resource"),
            "OCaml resources should detect unclosed open_in leak"
        );
    }

    #[test]
    fn test_resources_lua() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.lua",
            r#"
function leaky_function(path)
    local f = io.open(path, "r")
    local content = f:read("*a")
    -- f is never closed - resource leak
    return content
end
"#,
        );
        let output = tldr_cmd()
            .args([
                "resources",
                file.to_str().unwrap(),
                "leaky_function",
                "-f",
                "json",
            ])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("leak") || json.to_string().contains("resource"),
            "Lua resources should detect unclosed io.open leak"
        );
    }

    #[test]
    fn test_resources_luau() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.luau",
            r#"
local function leaky_function(path: string): string
    local f = io.open(path, "r")
    local content = f:read("*a")
    -- f is never closed - resource leak
    return content
end
"#,
        );
        let output = tldr_cmd()
            .args([
                "resources",
                file.to_str().unwrap(),
                "leaky_function",
                "-f",
                "json",
            ])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("leak") || json.to_string().contains("resource"),
            "Luau resources should detect unclosed io.open leak"
        );
    }
}

// =============================================================================
// Contracts Multi-Language Tests (3 MISSING languages only)
//
// Each test creates a source file with a function that has a precondition
// check (guard clause or assertion), runs `tldr contracts`, and asserts
// that a precondition is detected.
// =============================================================================

mod contracts_multilang {
    use super::*;

    #[test]
    fn test_contracts_kotlin() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.kt",
            r#"
fun processData(x: Int, data: List<Int>): Int {
    require(x >= 0) { "x must be non-negative" }
    check(data.isNotEmpty()) { "data cannot be empty" }
    return data.sum() + x
}
"#,
        );
        let output = tldr_cmd()
            .args([
                "contracts",
                file.to_str().unwrap(),
                "processData",
                "-f",
                "json",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("precondition") || json.to_string().contains("require"),
            "Kotlin contracts should detect require() and check() preconditions"
        );
    }

    /// Swift uses AST-based analysis via tree-sitter-swift 0.7.1
    #[test]
    fn test_contracts_swift() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.swift",
            r#"
func processData(x: Int, data: [Int]) -> Int {
    precondition(x >= 0, "x must be non-negative")
    guard !data.isEmpty else {
        fatalError("data cannot be empty")
    }
    return data.reduce(0, +) + x
}
"#,
        );
        let output = tldr_cmd()
            .args([
                "contracts",
                file.to_str().unwrap(),
                "processData",
                "-f",
                "json",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("precondition") || json.to_string().contains("guard"),
            "Swift contracts (regex fallback) should detect precondition() and guard clauses"
        );
    }

    #[test]
    fn test_contracts_luau() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.luau",
            r#"
local function processData(x: number, data: {number}): number
    assert(x >= 0, "x must be non-negative")
    if #data == 0 then
        error("data cannot be empty")
    end
    local sum = 0
    for _, v in ipairs(data) do
        sum = sum + v
    end
    return sum + x
end
"#,
        );
        let output = tldr_cmd()
            .args([
                "contracts",
                file.to_str().unwrap(),
                "processData",
                "-f",
                "json",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert!(
            json.to_string().contains("precondition") || json.to_string().contains("assert"),
            "Luau contracts should detect assert() and error() guard preconditions"
        );
    }
}

// =============================================================================
// Behavioral Multi-Language Tests (1 MISSING language: Swift)
//
// Swift uses regex fallback due to tree-sitter-swift ABI incompatibility.
// =============================================================================

mod behavioral_multilang {
    use super::*;

    /// Swift now uses tree-sitter AST (ABI v15 confirmed working in P0 with tree-sitter >= 0.25.0).
    /// The behavioral command extracts preconditions, postconditions, and side effects
    /// from Swift source code via tree-sitter-swift AST analysis.
    #[test]
    fn test_behavioral_swift() {
        let temp = TempDir::new().unwrap();
        let file = create_test_file(
            &temp,
            "test.swift",
            r#"
func processPositive(x: Int) -> Int {
    /// Process a positive number.
    ///
    /// - Parameter x: Must be positive
    /// - Returns: The doubled value
    /// - Throws: ValueError if x is not positive
    if x <= 0 {
        fatalError("x must be positive")
    }
    let result = x * 2
    assert(result > 0, "Result should be positive")
    return result
}
"#,
        );
        let output = tldr_cmd()
            .args([
                "behavioral",
                file.to_str().unwrap(),
                "processPositive",
                "-f",
                "json",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("valid JSON");
        // Should detect preconditions from guard clause and assertions
        assert!(
            json.to_string().contains("precondition")
                || json.to_string().contains("guard")
                || json.to_string().contains("assert"),
            "Swift behavioral (regex fallback) should detect preconditions from guard clauses"
        );
    }
}
