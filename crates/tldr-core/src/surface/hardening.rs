use std::collections::HashSet;
use std::fs;
use std::path::Path;

use proptest::prelude::*;
use tempfile::TempDir;

use super::extract_api_surface;

fn write_file(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

#[test]
fn test_behavioral_repo_shaped_surface_smoke_matrix() {
    let cases = vec![
        (
            "ruby",
            "lib/example.rb",
            "class Greeter\n  def hello(name)\n  end\nend\n",
        ),
        (
            "elixir",
            "lib/example.ex",
            "defmodule Example do\n  def hello(name), do: name\nend\n",
        ),
        (
            "java",
            "src/main/java/com/example/App.java",
            "public class App { public String hello(String name) { return name; } }\n",
        ),
        (
            "kotlin",
            "src/main/kotlin/com/example/App.kt",
            "class Greeter { fun hello(name: String): String = name }\n",
        ),
        (
            "csharp",
            "src/App.cs",
            "public class App { public string Hello(string name) => name; }\n",
        ),
        (
            "scala",
            "src/main/scala/com/example/App.scala",
            "object App { def hello(name: String): String = name }\n",
        ),
        (
            "php",
            "src/App.php",
            "<?php\nfunction hello($name) { return $name; }\n",
        ),
        (
            "swift",
            "Sources/App.swift",
            "public struct Greeter { public func hello(name: String) -> String { name } }\n",
        ),
        ("c", "include/api.h", "int hello(int name);\n"),
        (
            "cpp",
            "include/api.hpp",
            "class Greeter {\npublic:\n  void hello(int name);\n};\n",
        ),
        (
            "lua",
            "lua/app.lua",
            "local M = {}\nfunction M.hello(name)\n return name\nend\nreturn M\n",
        ),
    ];

    for (lang, rel, body) in cases {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), rel, body);
        if lang == "c" {
            write_file(dir.path(), "src/api.c", "int hidden(int x) { return x; }\n");
        }

        let surface =
            extract_api_surface(dir.path().to_str().unwrap(), Some(lang), false, None, None)
                .unwrap();

        assert_eq!(surface.language, lang);
        assert!(
            !surface.apis.is_empty(),
            "expected at least one API for {lang} fixture"
        );
    }
}

#[test]
fn test_invariant_surface_entries_are_unique_and_relative() {
    let fixtures = vec![
        (
            "ruby",
            "lib/example.rb",
            "class Greeter\n  def hello(name)\n  end\nend\n",
        ),
        (
            "java",
            "src/main/java/com/example/App.java",
            "public class App { public String hello(String name) { return name; } }\n",
        ),
        (
            "swift",
            "Sources/App.swift",
            "public func hello(name: String) -> String { name }\n",
        ),
        (
            "lua",
            "lua/app.lua",
            "local M = {}\nfunction M.hello(name)\n return name\nend\nreturn M\n",
        ),
    ];

    for (lang, rel, body) in fixtures {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), rel, body);

        let surface =
            extract_api_surface(dir.path().to_str().unwrap(), Some(lang), false, None, None)
                .unwrap();

        assert_eq!(
            surface.total,
            surface.apis.len(),
            "total mismatch for {lang}"
        );

        let mut seen = HashSet::new();
        for api in &surface.apis {
            assert!(
                !api.qualified_name.is_empty(),
                "empty qualified name for {lang}"
            );
            assert!(
                seen.insert(api.qualified_name.clone()),
                "duplicate API in {lang}"
            );
            if let Some(location) = &api.location {
                assert!(
                    !location.file.is_absolute(),
                    "expected relative location path for {lang}: {:?}",
                    location.file
                );
            }
        }
    }
}

#[test]
fn test_adversarial_visibility_and_export_cases_hold() {
    let dir = TempDir::new().unwrap();
    write_file(
        dir.path(),
        "lib/example.rb",
        "class Greeter\n  def hello(name)\n  end\n  private\n  def secret(name)\n  end\nend\n",
    );
    write_file(
        dir.path(),
        "lib/example.ex",
        "defmodule Example do\n  def hello(name), do: name\n  defp secret(name), do: name\nend\n",
    );
    write_file(dir.path(), "include/api.h", "int add(int a, int b);\n");
    write_file(dir.path(), "src/api.c", "int hidden(int x) { return x; }\n");
    write_file(
        dir.path(),
        "lua/app.lua",
        "local M = {}\nfunction M.hello(name)\n return name\nend\nlocal function hidden(name)\n return name\nend\nreturn M\n",
    );

    let ruby = extract_api_surface(
        dir.path().join("lib").to_str().unwrap(),
        Some("ruby"),
        false,
        None,
        None,
    )
    .unwrap();
    assert!(ruby
        .apis
        .iter()
        .any(|api| api.qualified_name.ends_with("hello")));
    assert!(!ruby
        .apis
        .iter()
        .any(|api| api.qualified_name.ends_with("secret")));

    let elixir = extract_api_surface(
        dir.path().join("lib").to_str().unwrap(),
        Some("elixir"),
        false,
        None,
        None,
    )
    .unwrap();
    assert!(elixir
        .apis
        .iter()
        .any(|api| api.qualified_name.ends_with("hello")));
    assert!(!elixir
        .apis
        .iter()
        .any(|api| api.qualified_name.ends_with("secret")));

    let c =
        extract_api_surface(dir.path().to_str().unwrap(), Some("c"), false, None, None).unwrap();
    assert!(c.apis.iter().any(|api| api.qualified_name.ends_with("add")));
    assert!(!c
        .apis
        .iter()
        .any(|api| api.qualified_name.ends_with("hidden")));

    let lua = extract_api_surface(
        dir.path().join("lua").to_str().unwrap(),
        Some("lua"),
        false,
        None,
        None,
    )
    .unwrap();
    assert!(lua
        .apis
        .iter()
        .any(|api| api.qualified_name.ends_with("hello")));
    assert!(!lua
        .apis
        .iter()
        .any(|api| api.qualified_name.ends_with("hidden")));
}

proptest! {
    #[test]
    fn prop_c_header_exports_generated_function(name in "[a-z][a-z0-9_]{0,8}") {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "include/api.h", &format!("int {name}(int value);\n"));
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("c"),
            false,
            None,
            None,
        )
        .unwrap();

        let expected = format!(".{}", name);
        let ok = surface
            .apis
            .iter()
            .any(|api| api.qualified_name.ends_with(&expected));
        prop_assert!(ok);
    }

    #[test]
    fn prop_lua_module_table_exports_generated_function(name in "[a-z][a-z0-9_]{0,8}") {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "lua/app.lua",
            &format!("local M = {{}}\nfunction M.{name}(value)\n return value\nend\nreturn M\n"),
        );
        let surface = extract_api_surface(
            dir.path().join("lua").to_str().unwrap(),
            Some("lua"),
            false,
            None,
            None,
        )
        .unwrap();

        let expected = format!(".{}", name);
        let ok = surface
            .apis
            .iter()
            .any(|api| api.qualified_name.ends_with(&expected));
        prop_assert!(ok);
    }
}
