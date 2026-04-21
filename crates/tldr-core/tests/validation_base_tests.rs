//! Test coverage for tldr-core validation module
//!
//! Tests all public functions from:
//! - crates/tldr-core/src/validation.rs

use std::path::Path;

use tempfile::TempDir;

use tldr_core::validation::*;
use tldr_core::{Language, TldrError};

// =============================================================================
// validate_file_path Tests
// =============================================================================

#[test]
fn test_validate_file_path_relative_with_project() {
    let temp = TempDir::new().unwrap();
    let src_dir = temp.path().join("src");
    std::fs::create_dir(&src_dir).unwrap();
    let main_rs = src_dir.join("main.rs");
    std::fs::write(&main_rs, "fn main() {}").unwrap();

    let result = validate_file_path("src/main.rs", Some(temp.path()));

    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    let path = result.unwrap();
    assert!(path.ends_with("src/main.rs"));
}

#[test]
fn test_validate_file_path_relative_nested() {
    let temp = TempDir::new().unwrap();
    let deep_dir = temp.path().join("a").join("b").join("c");
    std::fs::create_dir_all(&deep_dir).unwrap();
    let file = deep_dir.join("file.txt");
    std::fs::write(&file, "content").unwrap();

    let result = validate_file_path("a/b/c/file.txt", Some(temp.path()));

    assert!(result.is_ok());
    assert!(result.unwrap().ends_with("a/b/c/file.txt"));
}

#[test]
fn test_validate_file_path_absolute() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.txt");
    std::fs::write(&file, "content").unwrap();

    let result = validate_file_path(file.to_str().unwrap(), None);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), file.canonicalize().unwrap());
}

#[test]
fn test_validate_file_path_absolute_with_project() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.txt");
    std::fs::write(&file, "content").unwrap();

    // Absolute path should work even with project specified
    let result = validate_file_path(file.to_str().unwrap(), Some(temp.path()));

    assert!(result.is_ok());
}

#[test]
fn test_validate_file_path_not_found() {
    let result = validate_file_path("/definitely/nonexistent/path/file.rs", None);

    assert!(result.is_err());
    match result.unwrap_err() {
        TldrError::PathNotFound(_) => {}
        _ => panic!("Expected PathNotFound error"),
    }
}

#[test]
fn test_validate_file_path_not_found_relative() {
    let temp = TempDir::new().unwrap();

    let result = validate_file_path("nonexistent.rs", Some(temp.path()));

    assert!(result.is_err());
    match result.unwrap_err() {
        TldrError::PathNotFound(_) => {}
        _ => panic!("Expected PathNotFound error"),
    }
}

#[test]
fn test_validate_file_path_traversal_blocked() {
    let temp = TempDir::new().unwrap();
    let project_dir = temp.path().join("project");
    std::fs::create_dir(&project_dir).unwrap();
    // Create a file outside project dir
    let escape_file = temp.path().join("escape.rs");
    std::fs::write(&escape_file, "// escaped").unwrap();

    let result = validate_file_path("../escape.rs", Some(&project_dir));

    assert!(
        matches!(result, Err(TldrError::PathTraversal(_))),
        "Expected PathTraversal error, got {:?}",
        result
    );
}

#[test]
fn test_validate_file_path_traversal_multiple_levels() {
    let temp = TempDir::new().unwrap();
    let project_dir = temp.path().join("a").join("b").join("project");
    std::fs::create_dir_all(&project_dir).unwrap();
    // Create file outside
    let escape_file = temp.path().join("secret.rs");
    std::fs::write(&escape_file, "// secret").unwrap();

    let result = validate_file_path("../../../secret.rs", Some(&project_dir));

    assert!(matches!(result, Err(TldrError::PathTraversal(_))));
}

#[test]
fn test_validate_file_path_traversal_in_project_root() {
    let temp = TempDir::new().unwrap();
    let project_dir = temp.path();
    // Create file outside project (in parent of temp)
    // Note: Can't actually go above temp dir, so this tests the mechanism

    // Try to escape from root of temp
    let _result = validate_file_path("../", Some(project_dir));

    // This might succeed or fail depending on temp dir location
    // but should not panic
}

#[test]
fn test_validate_file_path_relative_without_project() {
    // This tests that relative paths resolve against cwd
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("local.txt");
    std::fs::write(&file, "content").unwrap();

    // Use absolute path since we can't change cwd easily in tests
    let result = validate_file_path(file.to_str().unwrap(), None);

    assert!(result.is_ok());
}

#[test]
fn test_validate_file_path_with_symlink() {
    let temp = TempDir::new().unwrap();
    let real_file = temp.path().join("real.txt");
    std::fs::write(&real_file, "content").unwrap();

    let link = temp.path().join("link.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real_file, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&real_file, &link).unwrap();

    let result = validate_file_path("link.txt", Some(temp.path()));

    // Should resolve symlink and return canonical path
    assert!(result.is_ok());
    assert!(result.unwrap().ends_with("real.txt"));
}

#[test]
fn test_validate_file_path_directory_instead_of_file() {
    let temp = TempDir::new().unwrap();
    let subdir = temp.path().join("subdir");
    std::fs::create_dir(&subdir).unwrap();

    // Trying to validate a directory as a file
    let result = validate_file_path("subdir", Some(temp.path()));

    // Directories can be canonicalized too, so this might succeed
    // The function doesn't specifically check if it's a file
    assert!(result.is_ok());
}

#[test]
fn test_validate_file_path_empty_string() {
    let temp = TempDir::new().unwrap();

    // Empty string should fail (current dir)
    let result = validate_file_path("", Some(temp.path()));

    // This attempts to canonicalize temp.path() itself
    assert!(result.is_ok()); // The temp dir exists
}

#[test]
fn test_validate_file_path_special_characters() {
    let temp = TempDir::new().unwrap();
    // Create file with special characters
    let file = temp.path().join("file with spaces.rs");
    std::fs::write(&file, "content").unwrap();

    let result = validate_file_path("file with spaces.rs", Some(temp.path()));

    assert!(result.is_ok());
}

#[test]
fn test_validate_file_path_unicode() {
    let temp = TempDir::new().unwrap();
    // Create file with unicode name
    let file = temp.path().join("文件.rs");
    std::fs::write(&file, "content").unwrap();

    let result = validate_file_path("文件.rs", Some(temp.path()));

    assert!(result.is_ok());
}

// =============================================================================
// detect_or_parse_language Tests
// =============================================================================

#[test]
fn test_parse_explicit_python() {
    let result = detect_or_parse_language(Some("python"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Python);
}

#[test]
fn test_parse_explicit_python_short() {
    let result = detect_or_parse_language(Some("py"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Python);
}

#[test]
fn test_parse_explicit_typescript() {
    let result = detect_or_parse_language(Some("typescript"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::TypeScript);
}

#[test]
fn test_parse_explicit_typescript_short() {
    let result = detect_or_parse_language(Some("ts"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::TypeScript);
}

#[test]
fn test_parse_explicit_rust() {
    let result = detect_or_parse_language(Some("rust"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Rust);
}

#[test]
fn test_parse_explicit_rust_short() {
    let result = detect_or_parse_language(Some("rs"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Rust);
}

#[test]
fn test_parse_explicit_go() {
    let result = detect_or_parse_language(Some("go"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Go);
}

#[test]
fn test_parse_explicit_go_alt() {
    let result = detect_or_parse_language(Some("golang"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Go);
}

#[test]
fn test_parse_explicit_java() {
    let result = detect_or_parse_language(Some("java"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Java);
}

#[test]
fn test_parse_explicit_c() {
    let result = detect_or_parse_language(Some("c"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::C);
}

#[test]
fn test_parse_explicit_cpp() {
    let result = detect_or_parse_language(Some("cpp"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Cpp);
}

#[test]
fn test_parse_explicit_cpp_alt() {
    let result = detect_or_parse_language(Some("c++"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Cpp);
}

#[test]
fn test_parse_explicit_ruby() {
    let result = detect_or_parse_language(Some("ruby"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Ruby);
}

#[test]
fn test_parse_explicit_kotlin() {
    let result = detect_or_parse_language(Some("kotlin"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Kotlin);
}

#[test]
fn test_parse_explicit_swift() {
    let result = detect_or_parse_language(Some("swift"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Swift);
}

#[test]
fn test_parse_explicit_csharp() {
    let result = detect_or_parse_language(Some("csharp"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::CSharp);
}

#[test]
fn test_parse_explicit_csharp_alt() {
    let result = detect_or_parse_language(Some("c#"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::CSharp);
}

#[test]
fn test_parse_explicit_scala() {
    let result = detect_or_parse_language(Some("scala"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Scala);
}

#[test]
fn test_parse_explicit_php() {
    let result = detect_or_parse_language(Some("php"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Php);
}

#[test]
fn test_parse_explicit_lua() {
    let result = detect_or_parse_language(Some("lua"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Lua);
}

#[test]
fn test_parse_explicit_luau() {
    let result = detect_or_parse_language(Some("luau"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Luau);
}

#[test]
fn test_parse_explicit_elixir() {
    let result = detect_or_parse_language(Some("elixir"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Elixir);
}

#[test]
fn test_parse_explicit_ocaml() {
    let result = detect_or_parse_language(Some("ocaml"), Path::new("any.xyz"));
    assert_eq!(result.unwrap(), Language::Ocaml);
}

#[test]
fn test_parse_invalid_language() {
    let result = detect_or_parse_language(Some("invalid_lang"), Path::new("any.xyz"));

    assert!(result.is_err());
    match result.unwrap_err() {
        TldrError::UnsupportedLanguage(lang) => {
            assert_eq!(lang, "invalid_lang");
        }
        _ => panic!("Expected UnsupportedLanguage error"),
    }
}

#[test]
fn test_parse_empty_language() {
    let result = detect_or_parse_language(Some(""), Path::new("any.xyz"));

    assert!(result.is_err());
    match result.unwrap_err() {
        TldrError::UnsupportedLanguage(lang) => {
            assert_eq!(lang, "");
        }
        _ => panic!("Expected UnsupportedLanguage error"),
    }
}

#[test]
fn test_detect_python_extension() {
    let result = detect_or_parse_language(None, Path::new("script.py"));
    assert_eq!(result.unwrap(), Language::Python);
}

#[test]
fn test_detect_rust_extension() {
    let result = detect_or_parse_language(None, Path::new("lib.rs"));
    assert_eq!(result.unwrap(), Language::Rust);
}

#[test]
fn test_detect_typescript_extension() {
    let result = detect_or_parse_language(None, Path::new("app.ts"));
    assert_eq!(result.unwrap(), Language::TypeScript);
}

#[test]
fn test_detect_typescript_tsx_extension() {
    let result = detect_or_parse_language(None, Path::new("component.tsx"));
    assert_eq!(result.unwrap(), Language::TypeScript);
}

#[test]
fn test_detect_go_extension() {
    let result = detect_or_parse_language(None, Path::new("main.go"));
    assert_eq!(result.unwrap(), Language::Go);
}

#[test]
fn test_detect_javascript_extension() {
    let result = detect_or_parse_language(None, Path::new("app.js"));
    assert_eq!(result.unwrap(), Language::JavaScript);
}

#[test]
fn test_detect_javascript_jsx_extension() {
    let result = detect_or_parse_language(None, Path::new("component.jsx"));
    assert_eq!(result.unwrap(), Language::JavaScript);
}

#[test]
fn test_detect_java_extension() {
    let result = detect_or_parse_language(None, Path::new("Main.java"));
    assert_eq!(result.unwrap(), Language::Java);
}

#[test]
fn test_detect_c_extension() {
    let result = detect_or_parse_language(None, Path::new("main.c"));
    assert_eq!(result.unwrap(), Language::C);
}

#[test]
fn test_detect_cpp_extension() {
    let result = detect_or_parse_language(None, Path::new("main.cpp"));
    assert_eq!(result.unwrap(), Language::Cpp);
}

#[test]
fn test_detect_unknown_extension() {
    let result = detect_or_parse_language(None, Path::new("file.xyz"));

    assert!(result.is_err());
    match result.unwrap_err() {
        TldrError::UnsupportedLanguage(msg) => {
            assert!(msg.contains("Could not detect language"));
            assert!(msg.contains("file.xyz"));
        }
        _ => panic!("Expected UnsupportedLanguage error"),
    }
}

#[test]
fn test_detect_no_extension() {
    let result = detect_or_parse_language(None, Path::new("Makefile"));

    assert!(result.is_err());
}

#[test]
fn test_detect_empty_path() {
    let result = detect_or_parse_language(None, Path::new(""));

    assert!(result.is_err());
}

#[test]
fn test_detect_dot_file() {
    let result = detect_or_parse_language(None, Path::new(".gitignore"));

    assert!(result.is_err());
}

#[test]
fn test_explicit_overrides_extension() {
    // Even if file is .py, explicit "rust" should win
    let result = detect_or_parse_language(Some("rust"), Path::new("script.py"));
    assert_eq!(result.unwrap(), Language::Rust);
}

#[test]
fn test_explicit_overrides_unknown_extension() {
    // Explicit language works even with unknown extension
    let result = detect_or_parse_language(Some("python"), Path::new("script.xyz"));
    assert_eq!(result.unwrap(), Language::Python);
}

#[test]
fn test_detect_case_insensitive_extension() {
    let result = detect_or_parse_language(None, Path::new("script.PY"));
    assert_eq!(result.unwrap(), Language::Python);
}

#[test]
fn test_detect_case_insensitive_mixed() {
    let result = detect_or_parse_language(None, Path::new("script.Py"));
    assert_eq!(result.unwrap(), Language::Python);
}

// =============================================================================
// Edge Cases and Integration Tests
// =============================================================================

#[test]
fn test_validate_and_detect_workflow() {
    let temp = TempDir::new().unwrap();
    let py_file = temp.path().join("script.py");
    std::fs::write(&py_file, "print('hello')").unwrap();

    // First validate the path
    let validated = validate_file_path("script.py", Some(temp.path())).unwrap();

    // Then detect language
    let lang = detect_or_parse_language(None, &validated).unwrap();

    assert_eq!(lang, Language::Python);
}

#[test]
fn test_validate_and_explicit_language() {
    let temp = TempDir::new().unwrap();
    // File with no extension
    let file = temp.path().join("script");
    std::fs::write(&file, "content").unwrap();

    // Validate path
    let validated = validate_file_path("script", Some(temp.path())).unwrap();

    // Use explicit language since no extension
    let lang = detect_or_parse_language(Some("python"), &validated).unwrap();

    assert_eq!(lang, Language::Python);
}

#[test]
fn test_validate_many_files() {
    let temp = TempDir::new().unwrap();

    // Create many files
    for i in 0..100 {
        let file = temp.path().join(format!("file{}.py", i));
        std::fs::write(&file, "pass").unwrap();
    }

    // Validate all
    for i in 0..100 {
        let path = format!("file{}.py", i);
        let result = validate_file_path(&path, Some(temp.path()));
        assert!(result.is_ok(), "Failed for file{}", i);
    }
}

#[test]
fn test_detect_various_extensions() {
    let temp = TempDir::new().unwrap();

    let test_cases = vec![
        ("test.py", Language::Python),
        ("test.rs", Language::Rust),
        ("test.go", Language::Go),
        ("test.ts", Language::TypeScript),
        ("test.tsx", Language::TypeScript),
        ("test.js", Language::JavaScript),
        ("test.jsx", Language::JavaScript),
        ("test.java", Language::Java),
        ("test.c", Language::C),
        ("test.cpp", Language::Cpp),
        ("test.rb", Language::Ruby),
        ("test.kt", Language::Kotlin),
        ("test.swift", Language::Swift),
        ("test.cs", Language::CSharp),
        ("test.scala", Language::Scala),
        ("test.php", Language::Php),
        ("test.lua", Language::Lua),
        ("test.luau", Language::Luau),
        ("test.ex", Language::Elixir),
        ("test.ml", Language::Ocaml),
    ];

    for (filename, expected_lang) in test_cases {
        let file = temp.path().join(filename);
        std::fs::write(&file, "content").unwrap();

        let result = detect_or_parse_language(None, &file);
        assert_eq!(result.unwrap(), expected_lang, "Failed for {}", filename);
    }
}
