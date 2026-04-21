use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use tldr_core::types::Language;

#[derive(Clone, Copy, Debug)]
pub struct TestFile {
    pub path: &'static str,
    pub body: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct SurfaceLanguageProfileCase {
    pub language: Language,
    pub target_rel: &'static str,
    pub files: &'static [TestFile],
    pub expected_public_symbols: &'static [&'static str],
    pub excluded_noise_symbols: &'static [&'static str],
}

const C_FILES: &[TestFile] = &[TestFile {
    path: "include/api.h",
    body: "int add(int a, int b);\n",
}];

const CPP_FILES: &[TestFile] = &[TestFile {
    path: "include/api.hpp",
    body: "class Greeter {\npublic:\n  void hello(int name);\n};\n",
}];

const CSHARP_FILES: &[TestFile] = &[TestFile {
    path: "src/Greeter.cs",
    body:
        "namespace Example { public class Greeter { public string Hello(string name) => name; } }\n",
}];

const ELIXIR_FILES: &[TestFile] = &[
    TestFile {
        path: "lib/example.ex",
        body: "defmodule Example do\n  def hello(name), do: name\nend\n",
    },
    TestFile {
        path: "examples/noise.ex",
        body: "defmodule Noise do\n  def example_noise(name), do: name\nend\n",
    },
    TestFile {
        path: "installer/mix/tasks/noise.ex",
        body: "defmodule Mix.Tasks.Noise do\n  def run(_args), do: :ok\nend\n",
    },
];

const GO_FILES: &[TestFile] = &[
    TestFile {
        path: "go.mod",
        body: "module example.com/mypkg\n\ngo 1.22\n",
    },
    TestFile {
        path: "add.go",
        body: "package mypkg\n\nfunc Add(a int, b int) int { return a + b }\n",
    },
];

const JAVA_FILES: &[TestFile] = &[TestFile {
    path: "src/main/java/com/example/Greeter.java",
    body: "package com.example;\npublic class Greeter {\n  public String hello(String name) { return name; }\n}\n",
}];

const JAVASCRIPT_FILES: &[TestFile] = &[
    TestFile {
        path: "index.js",
        body: "function greet(name) { return name; }\nmodule.exports = { greet };\n",
    },
    TestFile {
        path: "examples/demo.js",
        body: "function example_noise() { return 1; }\nmodule.exports = { example_noise };\n",
    },
    TestFile {
        path: "internal.test.js",
        body: "function suffix_noise() { return 1; }\nmodule.exports = { suffix_noise };\n",
    },
];

const KOTLIN_FILES: &[TestFile] = &[TestFile {
    path: "src/main/kotlin/com/example/Greeter.kt",
    body: "package com.example\nclass Greeter {\n  fun hello(name: String): String = name\n}\n",
}];

const LUA_FILES: &[TestFile] = &[TestFile {
    path: "lua/app.lua",
    body: "local M = {}\nfunction M.hello(name)\n  return name\nend\nreturn M\n",
}];

const PHP_FILES: &[TestFile] = &[
    TestFile {
        path: "src/Greeter.php",
        body: "<?php\nclass Greeter {\n    public function hello($name) { return $name; }\n}\n",
    },
    TestFile {
        path: "tests/Noise.php",
        body: "<?php\nclass Noise {\n    public function noise() { return 1; }\n}\n",
    },
];

const PYTHON_FILES: &[TestFile] = &[
    TestFile {
        path: "sample.py",
        body: "def public_api(name: str) -> str:\n    return name\n",
    },
    TestFile {
        path: "docs/tutorial.py",
        body: "def docs_api() -> int:\n    return 1\n",
    },
    TestFile {
        path: "examples/demo.py",
        body: "def example_api() -> int:\n    return 1\n",
    },
];

const RUBY_FILES: &[TestFile] = &[
    TestFile {
        path: "lib/example.rb",
        body: "class Greeter\n  def hello(name)\n    name\n  end\nend\n",
    },
    TestFile {
        path: "examples/noise.rb",
        body: "class Noise\n  def example_noise(name)\n    name\n  end\nend\n",
    },
];

const RUST_FILES: &[TestFile] = &[
    TestFile {
        path: "Cargo.toml",
        body: "[package]\nname = \"sample_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    },
    TestFile {
        path: "src/lib.rs",
        body: "pub fn public_api() {}\npub mod inner;\npub use inner::Helper;\n",
    },
    TestFile {
        path: "src/inner.rs",
        body: "pub struct Helper;\n",
    },
    TestFile {
        path: "examples/demo.rs",
        body: "pub fn example_api() {}\n",
    },
    TestFile {
        path: "benches/bench_api.rs",
        body: "pub fn bench_api() {}\n",
    },
    TestFile {
        path: "tests/integration.rs",
        body: "pub fn integration_api() {}\n",
    },
];

const SCALA_FILES: &[TestFile] = &[TestFile {
    path: "src/main/scala/com/example/Greeter.scala",
    body: "package com.example\nobject Greeter {\n  def hello(name: String): String = name\n}\n",
}];

const SWIFT_FILES: &[TestFile] = &[TestFile {
    path: "Sources/Greeter.swift",
    body: "public struct Greeter {\n    public init() {}\n    public func hello(name: String) -> String { return name }\n}\n",
}];

const TYPESCRIPT_FILES: &[TestFile] = &[
    TestFile {
        path: "src/index.ts",
        body: "export function greet(name: string): string { return name; }\n",
    },
    TestFile {
        path: "examples/demo.ts",
        body: "export function example_noise(): string { return \"x\"; }\n",
    },
    TestFile {
        path: "src/internal.test.ts",
        body: "export function suffix_noise(): string { return \"x\"; }\n",
    },
];

const PROFILES: &[SurfaceLanguageProfileCase] = &[
    SurfaceLanguageProfileCase {
        language: Language::C,
        target_rel: "",
        files: C_FILES,
        expected_public_symbols: &["add"],
        excluded_noise_symbols: &[],
    },
    SurfaceLanguageProfileCase {
        language: Language::Cpp,
        target_rel: "",
        files: CPP_FILES,
        expected_public_symbols: &["Greeter", "hello"],
        excluded_noise_symbols: &[],
    },
    SurfaceLanguageProfileCase {
        language: Language::CSharp,
        target_rel: "",
        files: CSHARP_FILES,
        expected_public_symbols: &["Greeter", "Hello"],
        excluded_noise_symbols: &[],
    },
    SurfaceLanguageProfileCase {
        language: Language::Elixir,
        target_rel: "",
        files: ELIXIR_FILES,
        expected_public_symbols: &["Example", "hello"],
        excluded_noise_symbols: &["example_noise", "Mix.Tasks.Noise"],
    },
    SurfaceLanguageProfileCase {
        language: Language::Go,
        target_rel: "",
        files: GO_FILES,
        expected_public_symbols: &["Add"],
        excluded_noise_symbols: &[],
    },
    SurfaceLanguageProfileCase {
        language: Language::Java,
        target_rel: "",
        files: JAVA_FILES,
        expected_public_symbols: &["Greeter", "hello"],
        excluded_noise_symbols: &[],
    },
    SurfaceLanguageProfileCase {
        language: Language::JavaScript,
        target_rel: "",
        files: JAVASCRIPT_FILES,
        expected_public_symbols: &["greet"],
        excluded_noise_symbols: &["example_noise", "suffix_noise"],
    },
    SurfaceLanguageProfileCase {
        language: Language::Kotlin,
        target_rel: "",
        files: KOTLIN_FILES,
        expected_public_symbols: &["Greeter", "hello"],
        excluded_noise_symbols: &[],
    },
    SurfaceLanguageProfileCase {
        language: Language::Lua,
        target_rel: "lua",
        files: LUA_FILES,
        expected_public_symbols: &["hello"],
        excluded_noise_symbols: &[],
    },
    SurfaceLanguageProfileCase {
        language: Language::Php,
        target_rel: "",
        files: PHP_FILES,
        expected_public_symbols: &["Greeter", "hello"],
        excluded_noise_symbols: &["Noise", "noise"],
    },
    SurfaceLanguageProfileCase {
        language: Language::Python,
        target_rel: "",
        files: PYTHON_FILES,
        expected_public_symbols: &["public_api"],
        excluded_noise_symbols: &["docs_api", "example_api"],
    },
    SurfaceLanguageProfileCase {
        language: Language::Ruby,
        target_rel: "",
        files: RUBY_FILES,
        expected_public_symbols: &["Greeter", "hello"],
        excluded_noise_symbols: &["example_noise"],
    },
    SurfaceLanguageProfileCase {
        language: Language::Rust,
        target_rel: "",
        files: RUST_FILES,
        expected_public_symbols: &["public_api", "Helper"],
        excluded_noise_symbols: &["example_api", "bench_api", "integration_api"],
    },
    SurfaceLanguageProfileCase {
        language: Language::Scala,
        target_rel: "",
        files: SCALA_FILES,
        expected_public_symbols: &["Greeter", "hello"],
        excluded_noise_symbols: &[],
    },
    SurfaceLanguageProfileCase {
        language: Language::Swift,
        target_rel: "",
        files: SWIFT_FILES,
        expected_public_symbols: &["Greeter", "hello"],
        excluded_noise_symbols: &[],
    },
    SurfaceLanguageProfileCase {
        language: Language::TypeScript,
        target_rel: "",
        files: TYPESCRIPT_FILES,
        expected_public_symbols: &["greet"],
        excluded_noise_symbols: &["example_noise", "suffix_noise"],
    },
];

pub fn all_surface_language_profiles() -> &'static [SurfaceLanguageProfileCase] {
    PROFILES
}

pub fn materialize_case(case: &SurfaceLanguageProfileCase) -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("create tempdir");
    for file in case.files {
        let path = temp_dir.path().join(file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(&path, file.body).expect("write test file");
    }

    let target_path = if case.target_rel.is_empty() {
        temp_dir.path().to_path_buf()
    } else {
        temp_dir.path().join(case.target_rel)
    };

    (temp_dir, target_path)
}

pub fn has_empty_path_segment(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| component.as_os_str().is_empty())
}
