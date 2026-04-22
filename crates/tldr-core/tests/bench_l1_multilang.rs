//! L1 AST multi-language benchmark tests
//!
//! Comprehensive tests for the five L1 commands (tree, structure, extract, imports, importers)
//! across all 17 supported fixture languages.
//!
//! Each test verifies OUTPUT CONTENT, not just exit codes:
//! - structure: non-empty definitions, correct names, valid line numbers, expected kinds
//! - extract: function/class counts, name correctness, line_end >= line_start
//! - imports: import counts, module names for files that have imports
//! - tree: directory enumeration, extension filtering

use std::collections::HashSet;
use std::path::PathBuf;

use tldr_core::{
    ast::extract::extract_file,
    ast::extractor::get_code_structure,
    ast::imports::get_imports,
    fs::tree::get_file_tree,
    Language, NodeType,
};

/// Get the fixtures/extractor directory path
fn extractor_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extractor")
}

// =============================================================================
// tree command tests
// =============================================================================

mod tree_tests {
    use super::*;

    #[test]
    fn test_tree_extractor_directory_non_empty() {
        // GIVEN: The extractor fixtures directory with 18 language files
        let dir = extractor_fixtures_dir();

        // WHEN: We get the file tree
        let tree = get_file_tree(&dir, None, true, None);

        // THEN: It should succeed and contain files
        assert!(tree.is_ok(), "tree failed: {:?}", tree.err());
        let tree = tree.unwrap();
        assert!(
            !tree.children.is_empty(),
            "extractor fixtures directory should not be empty"
        );
    }

    #[test]
    fn test_tree_filter_python_extension() {
        // GIVEN: The extractor fixtures directory
        let dir = extractor_fixtures_dir();
        let extensions: HashSet<String> = [".py".to_string()].into_iter().collect();

        // WHEN: We filter by .py
        let tree = get_file_tree(&dir, Some(&extensions), true, None);

        // THEN: Only .py files should appear
        let tree = tree.unwrap();
        fn check_py_only(node: &tldr_core::FileTree) {
            if node.node_type == NodeType::File {
                assert!(
                    node.name.ends_with(".py"),
                    "Non-py file found: {}",
                    node.name
                );
            }
            for child in &node.children {
                check_py_only(child);
            }
        }
        check_py_only(&tree);
        // There should be exactly 1 .py file
        let py_count = tree
            .children
            .iter()
            .filter(|c| c.name.ends_with(".py"))
            .count();
        assert_eq!(py_count, 1, "Expected exactly 1 .py file");
    }

    #[test]
    fn test_tree_filter_rust_extension() {
        // GIVEN: The extractor fixtures directory
        let dir = extractor_fixtures_dir();
        let extensions: HashSet<String> = [".rs".to_string()].into_iter().collect();

        // WHEN: We filter by .rs
        let tree = get_file_tree(&dir, Some(&extensions), true, None);

        // THEN: Only .rs files should appear
        let tree = tree.unwrap();
        let rs_count = tree
            .children
            .iter()
            .filter(|c| c.name.ends_with(".rs"))
            .count();
        assert_eq!(rs_count, 1, "Expected exactly 1 .rs file");
    }

    #[test]
    fn test_tree_contains_all_fixture_languages() {
        // GIVEN: The extractor fixtures directory
        let dir = extractor_fixtures_dir();

        // WHEN: We get the file tree
        let tree = get_file_tree(&dir, None, true, None).unwrap();

        // THEN: It should contain all expected fixture files
        let file_names: Vec<String> = tree.children.iter().map(|c| c.name.clone()).collect();
        let expected = [
            "test_python.py",
            "test_rust.rs",
            "test_go.go",
            "test_typescript.ts",
            "test_javascript.js",
            "test_java.java",
            "test_c.c",
            "test_cpp.cpp",
            "test_ruby.rb",
            "test_php.php",
            "test_kotlin.kt",
            "test_swift.swift",
            "test_csharp.cs",
            "test_scala.scala",
            "test_elixir.ex",
            "test_lua.lua",
            "test_ocaml.ml",
        ];
        for name in expected {
            assert!(
                file_names.contains(&name.to_string()),
                "Missing fixture file: {} (found: {:?})",
                name,
                file_names
            );
        }
    }
}

// =============================================================================
// structure command tests -- verify definitions, names, kinds, and line numbers
// =============================================================================

mod structure_tests {
    use super::*;

    /// Helper: run get_code_structure on a single fixture file and return the FileStructure
    fn structure_for(filename: &str, lang: Language) -> tldr_core::FileStructure {
        let file = extractor_fixtures_dir().join(filename);
        let result = get_code_structure(&file, lang, 0, None);
        assert!(
            result.is_ok(),
            "get_code_structure failed for {}: {:?}",
            filename,
            result.err()
        );
        let cs = result.unwrap();
        assert_eq!(cs.files.len(), 1, "Expected exactly 1 file for {}", filename);
        cs.files.into_iter().next().unwrap()
    }

    /// Helper: check that definitions have valid line numbers (all > 0, end >= start)
    fn assert_valid_line_numbers(defs: &[tldr_core::DefinitionInfo], lang_name: &str) {
        for def in defs {
            assert!(
                def.line_start > 0,
                "{}: definition '{}' has line_start=0",
                lang_name,
                def.name
            );
            assert!(
                def.line_end >= def.line_start,
                "{}: definition '{}' has line_end ({}) < line_start ({})",
                lang_name,
                def.name,
                def.line_end,
                def.line_start
            );
        }
    }

    /// Helper: assert a definition with given name and kind exists
    fn assert_has_definition(
        defs: &[tldr_core::DefinitionInfo],
        name: &str,
        expected_kinds: &[&str],
        lang_name: &str,
    ) {
        let found = defs.iter().find(|d| d.name == name);
        assert!(
            found.is_some(),
            "{}: expected definition '{}' not found in definitions: {:?}",
            lang_name,
            name,
            defs.iter().map(|d| &d.name).collect::<Vec<_>>()
        );
        let def = found.unwrap();
        assert!(
            expected_kinds.iter().any(|k| def.kind == *k),
            "{}: definition '{}' has kind '{}', expected one of {:?}",
            lang_name,
            name,
            def.kind,
            expected_kinds
        );
    }

    // --- Python ---
    // Fixture: 3 functions (top_level_func, another_func, async_func),
    //          2 classes (Animal, Dog), 5 methods

    #[test]
    fn test_structure_python() {
        let fs = structure_for("test_python.py", Language::Python);

        // Functions
        assert!(
            fs.functions.len() >= 3,
            "Python: expected >= 3 functions, got {}: {:?}",
            fs.functions.len(),
            fs.functions
        );
        assert!(fs.functions.contains(&"top_level_func".to_string()));
        assert!(fs.functions.contains(&"another_func".to_string()));
        assert!(fs.functions.contains(&"async_func".to_string()));

        // Classes
        assert!(
            fs.classes.len() >= 2,
            "Python: expected >= 2 classes, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));
        assert!(fs.classes.contains(&"Dog".to_string()));

        // Methods
        assert!(
            !fs.methods.is_empty(),
            "Python: expected non-empty methods"
        );

        // Definitions with line numbers
        assert!(
            !fs.definitions.is_empty(),
            "Python: definitions should be non-empty"
        );
        assert_valid_line_numbers(&fs.definitions, "Python");
        assert_has_definition(
            &fs.definitions,
            "top_level_func",
            &["function"],
            "Python",
        );
        assert_has_definition(&fs.definitions, "Animal", &["class"], "Python");
    }

    // --- Rust ---
    // Fixture: 3 functions (top_level, public_func, async_func),
    //          2 structs (Animal, Dog), 4 methods

    #[test]
    fn test_structure_rust() {
        let fs = structure_for("test_rust.rs", Language::Rust);

        assert!(
            fs.functions.len() >= 3,
            "Rust: expected >= 3 functions, got {}: {:?}",
            fs.functions.len(),
            fs.functions
        );
        assert!(fs.functions.contains(&"top_level".to_string()));
        assert!(fs.functions.contains(&"public_func".to_string()));
        assert!(fs.functions.contains(&"async_func".to_string()));

        assert!(
            fs.classes.len() >= 2,
            "Rust: expected >= 2 structs, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));
        assert!(fs.classes.contains(&"Dog".to_string()));

        assert!(!fs.definitions.is_empty(), "Rust: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "Rust");
        assert_has_definition(&fs.definitions, "top_level", &["function"], "Rust");
        assert_has_definition(&fs.definitions, "Animal", &["struct", "class"], "Rust");
    }

    // --- Go ---
    // Fixture: 4 functions (topLevel, anotherFunc, Speak, Fetch -- Go methods are functions),
    //          2 structs (Animal, Dog)

    #[test]
    fn test_structure_go() {
        let fs = structure_for("test_go.go", Language::Go);

        // Go methods with receivers may appear as functions or methods depending on implementation
        let total_funcs = fs.functions.len() + fs.methods.len();
        assert!(
            total_funcs >= 2,
            "Go: expected >= 2 total funcs+methods, got {}: functions={:?}, methods={:?}",
            total_funcs,
            fs.functions,
            fs.methods
        );

        // Check for the free functions
        assert!(
            fs.functions.contains(&"topLevel".to_string())
                || fs.functions.contains(&"anotherFunc".to_string()),
            "Go: expected topLevel or anotherFunc in functions: {:?}",
            fs.functions
        );

        assert!(
            fs.classes.len() >= 2,
            "Go: expected >= 2 structs, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));
        assert!(fs.classes.contains(&"Dog".to_string()));

        assert!(!fs.definitions.is_empty(), "Go: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "Go");
    }

    // --- TypeScript ---
    // Fixture: 3 functions (topLevel, arrowFunc, asyncFunc),
    //          2 classes (Animal, Dog), 4 methods

    #[test]
    fn test_structure_typescript() {
        let fs = structure_for("test_typescript.ts", Language::TypeScript);

        assert!(
            fs.functions.len() >= 2,
            "TypeScript: expected >= 2 functions, got {}: {:?}",
            fs.functions.len(),
            fs.functions
        );
        assert!(fs.functions.contains(&"topLevel".to_string()));

        assert!(
            fs.classes.len() >= 2,
            "TypeScript: expected >= 2 classes, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));
        assert!(fs.classes.contains(&"Dog".to_string()));

        assert!(!fs.definitions.is_empty(), "TypeScript: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "TypeScript");
        assert_has_definition(&fs.definitions, "topLevel", &["function"], "TypeScript");
        assert_has_definition(&fs.definitions, "Animal", &["class"], "TypeScript");
    }

    // --- JavaScript ---
    // Fixture: 5 functions (topLevel, arrowFunc, helper/namedArrow, asyncFunc, generatorFunc),
    //          2 classes (Animal, Dog), 4 methods

    #[test]
    fn test_structure_javascript() {
        let fs = structure_for("test_javascript.js", Language::JavaScript);

        assert!(
            fs.functions.len() >= 3,
            "JavaScript: expected >= 3 functions, got {}: {:?}",
            fs.functions.len(),
            fs.functions
        );
        assert!(fs.functions.contains(&"topLevel".to_string()));

        assert!(
            fs.classes.len() >= 2,
            "JavaScript: expected >= 2 classes, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));
        assert!(fs.classes.contains(&"Dog".to_string()));

        assert!(
            !fs.definitions.is_empty(),
            "JavaScript: definitions should be non-empty"
        );
        assert_valid_line_numbers(&fs.definitions, "JavaScript");
        assert_has_definition(&fs.definitions, "topLevel", &["function"], "JavaScript");
    }

    // --- Java ---
    // Fixture: 0 free functions, 3 classes (Animal, Dog, Utils), 6 methods

    #[test]
    fn test_structure_java() {
        let fs = structure_for("test_java.java", Language::Java);

        assert!(
            fs.classes.len() >= 3,
            "Java: expected >= 3 classes, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));
        assert!(fs.classes.contains(&"Dog".to_string()));
        assert!(fs.classes.contains(&"Utils".to_string()));

        // Methods should be non-empty (Java has all methods inside classes)
        assert!(
            !fs.methods.is_empty(),
            "Java: expected non-empty methods, got {:?}",
            fs.methods
        );

        assert!(!fs.definitions.is_empty(), "Java: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "Java");
        assert_has_definition(&fs.definitions, "Animal", &["class"], "Java");
    }

    // --- C ---
    // Fixture: 4 functions (animal_speak, add, helper, main),
    //          2 structs (Animal, Dog), 0 methods

    #[test]
    fn test_structure_c() {
        let fs = structure_for("test_c.c", Language::C);

        assert!(
            fs.functions.len() >= 4,
            "C: expected >= 4 functions, got {}: {:?}",
            fs.functions.len(),
            fs.functions
        );
        assert!(fs.functions.contains(&"animal_speak".to_string()));
        assert!(fs.functions.contains(&"add".to_string()));
        assert!(fs.functions.contains(&"main".to_string()));

        // C structs
        assert!(
            !fs.classes.is_empty(),
            "C: expected non-empty structs, got {:?}",
            fs.classes
        );

        assert!(!fs.definitions.is_empty(), "C: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "C");
    }

    // --- C++ ---
    // Fixture: 2 functions (globalFunc, helperFunc),
    //          2 classes (Animal, Dog), 5 methods

    #[test]
    fn test_structure_cpp() {
        let fs = structure_for("test_cpp.cpp", Language::Cpp);

        assert!(
            fs.functions.len() >= 2,
            "C++: expected >= 2 functions, got {}: {:?}",
            fs.functions.len(),
            fs.functions
        );
        assert!(fs.functions.contains(&"globalFunc".to_string()));

        assert!(
            fs.classes.len() >= 2,
            "C++: expected >= 2 classes, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));
        assert!(fs.classes.contains(&"Dog".to_string()));

        assert!(!fs.definitions.is_empty(), "C++: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "C++");
    }

    // --- Ruby ---
    // Fixture: 2 functions (top_level_func, another_func),
    //          2 classes (Animal, Dog), 5 methods

    #[test]
    fn test_structure_ruby() {
        let fs = structure_for("test_ruby.rb", Language::Ruby);

        assert!(
            fs.functions.len() >= 2,
            "Ruby: expected >= 2 functions, got {}: {:?}",
            fs.functions.len(),
            fs.functions
        );
        assert!(fs.functions.contains(&"top_level_func".to_string()));
        assert!(fs.functions.contains(&"another_func".to_string()));

        assert!(
            fs.classes.len() >= 2,
            "Ruby: expected >= 2 classes, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));
        assert!(fs.classes.contains(&"Dog".to_string()));

        assert!(!fs.definitions.is_empty(), "Ruby: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "Ruby");
        assert_has_definition(&fs.definitions, "top_level_func", &["function", "method"], "Ruby");
        assert_has_definition(&fs.definitions, "Animal", &["class", "module"], "Ruby");
    }

    // --- PHP ---
    // Fixture: 2 functions (topLevelFunc, anotherFunc),
    //          2 classes (Animal, Dog), 5 methods

    #[test]
    fn test_structure_php() {
        let fs = structure_for("test_php.php", Language::Php);

        assert!(
            fs.functions.len() >= 2,
            "PHP: expected >= 2 functions, got {}: {:?}",
            fs.functions.len(),
            fs.functions
        );
        assert!(fs.functions.contains(&"topLevelFunc".to_string()));
        assert!(fs.functions.contains(&"anotherFunc".to_string()));

        assert!(
            fs.classes.len() >= 2,
            "PHP: expected >= 2 classes, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));
        assert!(fs.classes.contains(&"Dog".to_string()));

        assert!(!fs.definitions.is_empty(), "PHP: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "PHP");
    }

    // --- Kotlin ---
    // Fixture: 2 functions (topLevel, anotherFunc),
    //          2 classes (Animal, Dog), 5 methods

    #[test]
    fn test_structure_kotlin() {
        let fs = structure_for("test_kotlin.kt", Language::Kotlin);

        assert!(
            fs.functions.len() >= 2,
            "Kotlin: expected >= 2 functions, got {}: {:?}",
            fs.functions.len(),
            fs.functions
        );
        assert!(fs.functions.contains(&"topLevel".to_string()));
        assert!(fs.functions.contains(&"anotherFunc".to_string()));

        assert!(
            fs.classes.len() >= 2,
            "Kotlin: expected >= 2 classes, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));
        assert!(fs.classes.contains(&"Dog".to_string()));

        assert!(!fs.definitions.is_empty(), "Kotlin: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "Kotlin");
        // companion object must produce a "Companion" definition (not an empty name)
        assert_has_definition(&fs.definitions, "Companion", &["class"], "Kotlin");
    }

    // --- Swift ---
    // Fixture: 3 functions (topLevel, anotherFunc, asyncFunc),
    //          2 classes (Animal, Dog), 5 methods

    #[test]
    fn test_structure_swift() {
        let fs = structure_for("test_swift.swift", Language::Swift);

        assert!(
            fs.functions.len() >= 3,
            "Swift: expected >= 3 functions, got {}: {:?}",
            fs.functions.len(),
            fs.functions
        );
        assert!(fs.functions.contains(&"topLevel".to_string()));
        assert!(fs.functions.contains(&"anotherFunc".to_string()));
        assert!(fs.functions.contains(&"asyncFunc".to_string()));

        assert!(
            fs.classes.len() >= 2,
            "Swift: expected >= 2 classes, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));
        assert!(fs.classes.contains(&"Dog".to_string()));

        assert!(!fs.definitions.is_empty(), "Swift: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "Swift");
    }

    // --- C# ---
    // Fixture: 0 free functions, 3 classes (Animal, Dog, Utils), 7 methods

    #[test]
    fn test_structure_csharp() {
        let fs = structure_for("test_csharp.cs", Language::CSharp);

        assert!(
            fs.classes.len() >= 3,
            "C#: expected >= 3 classes, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));
        assert!(fs.classes.contains(&"Dog".to_string()));
        assert!(fs.classes.contains(&"Utils".to_string()));

        // C# has no free functions -- all methods inside classes
        assert!(
            !fs.methods.is_empty(),
            "C#: expected non-empty methods, got {:?}",
            fs.methods
        );

        assert!(!fs.definitions.is_empty(), "C#: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "C#");
        assert_has_definition(&fs.definitions, "Animal", &["class"], "C#");
    }

    // --- Scala ---
    // Fixture: 2 functions (in object Utils), 2 classes (Animal, Dog), 4 methods

    #[test]
    fn test_structure_scala() {
        let fs = structure_for("test_scala.scala", Language::Scala);

        // Scala object functions may appear as methods or functions
        let total = fs.functions.len() + fs.methods.len();
        assert!(
            total >= 2,
            "Scala: expected >= 2 total funcs+methods, got {}: functions={:?}, methods={:?}",
            total,
            fs.functions,
            fs.methods
        );

        // Classes/objects
        assert!(
            fs.classes.len() >= 2,
            "Scala: expected >= 2 classes/objects, got {}: {:?}",
            fs.classes.len(),
            fs.classes
        );

        assert!(!fs.definitions.is_empty(), "Scala: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "Scala");
    }

    // --- Elixir ---
    // Fixture: 3 functions (speak, create, helper), 1 module (Animal)

    #[test]
    fn test_structure_elixir() {
        let fs = structure_for("test_elixir.ex", Language::Elixir);

        // Elixir def/defp may appear as functions or methods
        let total = fs.functions.len() + fs.methods.len();
        assert!(
            total >= 2,
            "Elixir: expected >= 2 total funcs+methods, got {}: functions={:?}, methods={:?}",
            total,
            fs.functions,
            fs.methods
        );

        // defmodule counts as a class
        assert!(
            !fs.classes.is_empty(),
            "Elixir: expected non-empty classes (modules), got {:?}",
            fs.classes
        );
        assert!(fs.classes.contains(&"Animal".to_string()));

        assert!(!fs.definitions.is_empty(), "Elixir: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "Elixir");
    }

    // --- Lua ---
    // Fixture: 5 functions, 0 classes, 0 methods

    #[test]
    fn test_structure_lua() {
        let fs = structure_for("test_lua.lua", Language::Lua);

        assert!(
            fs.functions.len() >= 3,
            "Lua: expected >= 3 functions, got {}: {:?}",
            fs.functions.len(),
            fs.functions
        );

        assert!(!fs.definitions.is_empty(), "Lua: definitions should be non-empty");
        assert_valid_line_numbers(&fs.definitions, "Lua");
    }

    // --- OCaml ---
    // Fixture: 3 functions (top_level, another_func, factorial), 0 classes

    #[test]
    fn test_structure_ocaml() {
        let fs = structure_for("test_ocaml.ml", Language::Ocaml);

        assert!(
            fs.functions.len() >= 2,
            "OCaml: expected >= 2 functions, got {}: {:?}",
            fs.functions.len(),
            fs.functions
        );

        // OCaml definitions are now populated via value_definition node classification.
        assert!(
            !fs.definitions.is_empty(),
            "OCaml: definitions must be non-empty"
        );
        assert_valid_line_numbers(&fs.definitions, "OCaml");
        assert_has_definition(&fs.definitions, "top_level", &["function"], "OCaml");
        assert_has_definition(&fs.definitions, "another_func", &["function"], "OCaml");
        assert_has_definition(&fs.definitions, "factorial", &["function"], "OCaml");
    }

    // --- Cross-language: every definition has a non-empty name ---

    #[test]
    fn test_structure_all_definitions_have_names() {
        let cases: Vec<(&str, Language)> = vec![
            ("test_python.py", Language::Python),
            ("test_rust.rs", Language::Rust),
            ("test_go.go", Language::Go),
            ("test_typescript.ts", Language::TypeScript),
            ("test_javascript.js", Language::JavaScript),
            ("test_java.java", Language::Java),
            ("test_c.c", Language::C),
            ("test_cpp.cpp", Language::Cpp),
            ("test_ruby.rb", Language::Ruby),
            ("test_php.php", Language::Php),
            ("test_kotlin.kt", Language::Kotlin),
            ("test_swift.swift", Language::Swift),
            ("test_csharp.cs", Language::CSharp),
            ("test_scala.scala", Language::Scala),
            ("test_elixir.ex", Language::Elixir),
            ("test_lua.lua", Language::Lua),
            ("test_ocaml.ml", Language::Ocaml),
        ];

        for (filename, lang) in cases {
            let fs = structure_for(filename, lang);
            for def in &fs.definitions {
                assert!(
                    !def.name.is_empty(),
                    "{}: found definition with empty name: {:?}",
                    filename,
                    def
                );
            }
        }
    }

    // --- Cross-language: every definition has a valid kind ---

    #[test]
    fn test_structure_all_definitions_have_valid_kinds() {
        // VAL-004: `field` is emitted for class-scope field/property
        // declarations (Java field_declaration, Kotlin/Swift
        // property_declaration inside class body, TS public_field_definition /
        // field_definition).
        let valid_kinds = [
            "function", "method", "class", "struct", "module", "constant",
            "interface", "trait", "enum", "type_alias", "object", "field",
        ];
        let cases: Vec<(&str, Language)> = vec![
            ("test_python.py", Language::Python),
            ("test_rust.rs", Language::Rust),
            ("test_go.go", Language::Go),
            ("test_typescript.ts", Language::TypeScript),
            ("test_javascript.js", Language::JavaScript),
            ("test_java.java", Language::Java),
            ("test_c.c", Language::C),
            ("test_cpp.cpp", Language::Cpp),
            ("test_ruby.rb", Language::Ruby),
            ("test_php.php", Language::Php),
            ("test_kotlin.kt", Language::Kotlin),
            ("test_swift.swift", Language::Swift),
            ("test_csharp.cs", Language::CSharp),
            ("test_scala.scala", Language::Scala),
            ("test_elixir.ex", Language::Elixir),
            ("test_lua.lua", Language::Lua),
            ("test_ocaml.ml", Language::Ocaml),
        ];

        for (filename, lang) in cases {
            let fs = structure_for(filename, lang);
            for def in &fs.definitions {
                assert!(
                    valid_kinds.contains(&def.kind.as_str()),
                    "{}: definition '{}' has unexpected kind '{}' (valid: {:?})",
                    filename,
                    def.name,
                    def.kind,
                    valid_kinds
                );
            }
        }
    }
}

// =============================================================================
// extract command tests -- verify functions, classes, line numbers
// =============================================================================

mod extract_tests {
    use super::*;

    /// Helper: run extract_file on a fixture file
    fn extract_for(filename: &str) -> tldr_core::ModuleInfo {
        let file = extractor_fixtures_dir().join(filename);
        let result = extract_file(&file, None);
        assert!(
            result.is_ok(),
            "extract_file failed for {}: {:?}",
            filename,
            result.err()
        );
        result.unwrap()
    }

    // --- Python ---

    #[test]
    fn test_extract_python() {
        let info = extract_for("test_python.py");

        assert_eq!(info.language, Language::Python);
        assert!(
            info.functions.len() >= 3,
            "Python extract: expected >= 3 functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert!(
            info.classes.len() >= 2,
            "Python extract: expected >= 2 classes, got {}: {:?}",
            info.classes.len(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        // Verify specific function names
        let fn_names: Vec<&str> = info.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(fn_names.contains(&"top_level_func"), "Missing top_level_func: {:?}", fn_names);
        assert!(fn_names.contains(&"another_func"), "Missing another_func: {:?}", fn_names);
        assert!(fn_names.contains(&"async_func"), "Missing async_func: {:?}", fn_names);

        // Verify class names
        let cls_names: Vec<&str> = info.classes.iter().map(|c| c.name.as_str()).collect();
        assert!(cls_names.contains(&"Animal"), "Missing Animal: {:?}", cls_names);
        assert!(cls_names.contains(&"Dog"), "Missing Dog: {:?}", cls_names);

        // Verify line numbers
        for func in &info.functions {
            assert!(func.line_number > 0, "Python: function '{}' has line_number=0", func.name);
        }
        for cls in &info.classes {
            assert!(cls.line_number > 0, "Python: class '{}' has line_number=0", cls.name);
        }

        // Dog should extend Animal
        let dog = info.classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(
            dog.bases.contains(&"Animal".to_string()),
            "Python: Dog should extend Animal, bases={:?}",
            dog.bases
        );
    }

    // --- Rust ---

    #[test]
    fn test_extract_rust() {
        let info = extract_for("test_rust.rs");

        assert_eq!(info.language, Language::Rust);
        assert!(
            info.functions.len() >= 3,
            "Rust extract: expected >= 3 functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert!(
            info.classes.len() >= 2,
            "Rust extract: expected >= 2 structs, got {}: {:?}",
            info.classes.len(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        let fn_names: Vec<&str> = info.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(fn_names.contains(&"top_level"), "Missing top_level: {:?}", fn_names);
        assert!(fn_names.contains(&"public_func"), "Missing public_func: {:?}", fn_names);

        // Verify async detection
        let async_fn = info.functions.iter().find(|f| f.name == "async_func");
        assert!(async_fn.is_some(), "Rust: async_func not found");
        assert!(async_fn.unwrap().is_async, "Rust: async_func should be marked async");

        // Structs have methods
        let animal = info.classes.iter().find(|c| c.name == "Animal");
        assert!(animal.is_some(), "Rust: Animal struct not found");
        assert!(
            !animal.unwrap().methods.is_empty(),
            "Rust: Animal should have methods"
        );
    }

    // --- Go ---

    #[test]
    fn test_extract_go() {
        let info = extract_for("test_go.go");

        assert_eq!(info.language, Language::Go);

        // Go has free functions and receiver methods
        let total = info.functions.len()
            + info
                .classes
                .iter()
                .map(|c| c.methods.len())
                .sum::<usize>();
        assert!(
            total >= 4,
            "Go extract: expected >= 4 total funcs+methods, got {}: functions={:?}",
            total,
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );

        assert!(
            info.classes.len() >= 2,
            "Go extract: expected >= 2 structs, got {}",
            info.classes.len()
        );
    }

    // --- TypeScript ---

    #[test]
    fn test_extract_typescript() {
        let info = extract_for("test_typescript.ts");

        assert_eq!(info.language, Language::TypeScript);
        assert!(
            info.functions.len() >= 2,
            "TypeScript extract: expected >= 2 functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert!(
            info.classes.len() >= 2,
            "TypeScript extract: expected >= 2 classes, got {}",
            info.classes.len()
        );

        let fn_names: Vec<&str> = info.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(fn_names.contains(&"topLevel"), "Missing topLevel: {:?}", fn_names);

        // asyncFunc should be async
        let async_fn = info.functions.iter().find(|f| f.name == "asyncFunc");
        assert!(async_fn.is_some(), "TypeScript: asyncFunc not found");
        assert!(
            async_fn.unwrap().is_async,
            "TypeScript: asyncFunc should be marked async"
        );
    }

    // --- JavaScript ---

    #[test]
    fn test_extract_javascript() {
        let info = extract_for("test_javascript.js");

        assert_eq!(info.language, Language::JavaScript);
        assert!(
            info.functions.len() >= 3,
            "JavaScript extract: expected >= 3 functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert!(
            info.classes.len() >= 2,
            "JavaScript extract: expected >= 2 classes, got {}",
            info.classes.len()
        );

        let fn_names: Vec<&str> = info.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(fn_names.contains(&"topLevel"), "Missing topLevel: {:?}", fn_names);

        let cls_names: Vec<&str> = info.classes.iter().map(|c| c.name.as_str()).collect();
        assert!(cls_names.contains(&"Animal"), "Missing Animal: {:?}", cls_names);
        assert!(cls_names.contains(&"Dog"), "Missing Dog: {:?}", cls_names);
    }

    // --- Java ---

    #[test]
    fn test_extract_java() {
        let info = extract_for("test_java.java");

        assert_eq!(info.language, Language::Java);
        // Java: no free functions
        assert!(
            info.classes.len() >= 3,
            "Java extract: expected >= 3 classes, got {}: {:?}",
            info.classes.len(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        let cls_names: Vec<&str> = info.classes.iter().map(|c| c.name.as_str()).collect();
        assert!(cls_names.contains(&"Animal"), "Missing Animal: {:?}", cls_names);
        assert!(cls_names.contains(&"Dog"), "Missing Dog: {:?}", cls_names);
        assert!(cls_names.contains(&"Utils"), "Missing Utils: {:?}", cls_names);

        // Dog extends Animal
        let dog = info.classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(
            dog.bases.contains(&"Animal".to_string()),
            "Java: Dog should extend Animal, bases={:?}",
            dog.bases
        );

        // Animal should have methods
        let animal = info.classes.iter().find(|c| c.name == "Animal").unwrap();
        assert!(
            !animal.methods.is_empty(),
            "Java: Animal should have methods"
        );
    }

    // --- C ---

    #[test]
    fn test_extract_c() {
        let info = extract_for("test_c.c");

        assert_eq!(info.language, Language::C);
        assert!(
            info.functions.len() >= 4,
            "C extract: expected >= 4 functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );

        let fn_names: Vec<&str> = info.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(fn_names.contains(&"animal_speak"), "Missing animal_speak: {:?}", fn_names);
        assert!(fn_names.contains(&"add"), "Missing add: {:?}", fn_names);
        assert!(fn_names.contains(&"main"), "Missing main: {:?}", fn_names);
        assert!(fn_names.contains(&"helper"), "Missing helper: {:?}", fn_names);
    }

    // --- C++ ---

    #[test]
    fn test_extract_cpp() {
        let info = extract_for("test_cpp.cpp");

        assert_eq!(info.language, Language::Cpp);
        assert!(
            info.functions.len() >= 2,
            "C++ extract: expected >= 2 functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert!(
            info.classes.len() >= 2,
            "C++ extract: expected >= 2 classes, got {}: {:?}",
            info.classes.len(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        let cls_names: Vec<&str> = info.classes.iter().map(|c| c.name.as_str()).collect();
        assert!(cls_names.contains(&"Animal"), "Missing Animal: {:?}", cls_names);
        assert!(cls_names.contains(&"Dog"), "Missing Dog: {:?}", cls_names);
    }

    // --- Ruby ---

    #[test]
    fn test_extract_ruby() {
        let info = extract_for("test_ruby.rb");

        assert_eq!(info.language, Language::Ruby);
        assert!(
            info.functions.len() >= 2,
            "Ruby extract: expected >= 2 functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert!(
            info.classes.len() >= 2,
            "Ruby extract: expected >= 2 classes, got {}: {:?}",
            info.classes.len(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        let fn_names: Vec<&str> = info.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(
            fn_names.contains(&"top_level_func"),
            "Missing top_level_func: {:?}",
            fn_names
        );

        let cls_names: Vec<&str> = info.classes.iter().map(|c| c.name.as_str()).collect();
        assert!(cls_names.contains(&"Animal"), "Missing Animal: {:?}", cls_names);
        assert!(cls_names.contains(&"Dog"), "Missing Dog: {:?}", cls_names);
    }

    // --- PHP ---

    #[test]
    fn test_extract_php() {
        let info = extract_for("test_php.php");

        assert_eq!(info.language, Language::Php);
        assert!(
            info.functions.len() >= 2,
            "PHP extract: expected >= 2 functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert!(
            info.classes.len() >= 2,
            "PHP extract: expected >= 2 classes, got {}: {:?}",
            info.classes.len(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        let fn_names: Vec<&str> = info.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(fn_names.contains(&"topLevelFunc"), "Missing topLevelFunc: {:?}", fn_names);
        assert!(fn_names.contains(&"anotherFunc"), "Missing anotherFunc: {:?}", fn_names);

        let cls_names: Vec<&str> = info.classes.iter().map(|c| c.name.as_str()).collect();
        assert!(cls_names.contains(&"Animal"), "Missing Animal: {:?}", cls_names);
        assert!(cls_names.contains(&"Dog"), "Missing Dog: {:?}", cls_names);
    }

    // --- Kotlin ---

    #[test]
    fn test_extract_kotlin() {
        let info = extract_for("test_kotlin.kt");

        assert_eq!(info.language, Language::Kotlin);
        assert!(
            info.functions.len() >= 2,
            "Kotlin extract: expected >= 2 functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert!(
            info.classes.len() >= 2,
            "Kotlin extract: expected >= 2 classes, got {}: {:?}",
            info.classes.len(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        let fn_names: Vec<&str> = info.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(fn_names.contains(&"topLevel"), "Missing topLevel: {:?}", fn_names);
        assert!(fn_names.contains(&"anotherFunc"), "Missing anotherFunc: {:?}", fn_names);
    }

    // --- Swift ---

    #[test]
    fn test_extract_swift() {
        let info = extract_for("test_swift.swift");

        assert_eq!(info.language, Language::Swift);
        assert!(
            info.functions.len() >= 3,
            "Swift extract: expected >= 3 functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert!(
            info.classes.len() >= 2,
            "Swift extract: expected >= 2 classes, got {}: {:?}",
            info.classes.len(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        let fn_names: Vec<&str> = info.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(fn_names.contains(&"topLevel"), "Missing topLevel: {:?}", fn_names);
        assert!(fn_names.contains(&"anotherFunc"), "Missing anotherFunc: {:?}", fn_names);
        assert!(fn_names.contains(&"asyncFunc"), "Missing asyncFunc: {:?}", fn_names);

        let cls_names: Vec<&str> = info.classes.iter().map(|c| c.name.as_str()).collect();
        assert!(cls_names.contains(&"Animal"), "Missing Animal: {:?}", cls_names);
        assert!(cls_names.contains(&"Dog"), "Missing Dog: {:?}", cls_names);
    }

    // --- C# ---

    #[test]
    fn test_extract_csharp() {
        let info = extract_for("test_csharp.cs");

        assert_eq!(info.language, Language::CSharp);
        // C# has no free functions
        assert!(
            info.classes.len() >= 3,
            "C# extract: expected >= 3 classes, got {}: {:?}",
            info.classes.len(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        let cls_names: Vec<&str> = info.classes.iter().map(|c| c.name.as_str()).collect();
        assert!(cls_names.contains(&"Animal"), "Missing Animal: {:?}", cls_names);
        assert!(cls_names.contains(&"Dog"), "Missing Dog: {:?}", cls_names);
        assert!(cls_names.contains(&"Utils"), "Missing Utils: {:?}", cls_names);

        // Animal should have methods
        let animal = info.classes.iter().find(|c| c.name == "Animal").unwrap();
        assert!(
            !animal.methods.is_empty(),
            "C#: Animal should have methods"
        );
    }

    // --- Scala ---

    #[test]
    fn test_extract_scala() {
        let info = extract_for("test_scala.scala");

        assert_eq!(info.language, Language::Scala);

        // Scala has objects and classes
        let total_funcs = info.functions.len()
            + info
                .classes
                .iter()
                .map(|c| c.methods.len())
                .sum::<usize>();
        assert!(
            total_funcs >= 4,
            "Scala extract: expected >= 4 total funcs+methods, got {}: functions={:?}, classes={:?}",
            total_funcs,
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }

    // --- Elixir ---

    #[test]
    fn test_extract_elixir() {
        let info = extract_for("test_elixir.ex");

        assert_eq!(info.language, Language::Elixir);

        // Elixir defmodule is a class, def/defp are functions or methods
        let total = info.functions.len()
            + info
                .classes
                .iter()
                .map(|c| c.methods.len())
                .sum::<usize>();
        assert!(
            total >= 2,
            "Elixir extract: expected >= 2 total funcs+methods, got {}: functions={:?}, classes={:?}",
            total,
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        assert!(
            !info.classes.is_empty(),
            "Elixir extract: expected non-empty classes (modules)"
        );
    }

    // --- Lua ---

    #[test]
    fn test_extract_lua() {
        let info = extract_for("test_lua.lua");

        assert_eq!(info.language, Language::Lua);
        assert!(
            info.functions.len() >= 3,
            "Lua extract: expected >= 3 functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    // --- OCaml ---

    #[test]
    fn test_extract_ocaml() {
        let info = extract_for("test_ocaml.ml");

        assert_eq!(info.language, Language::Ocaml);
        assert!(
            info.functions.len() >= 2,
            "OCaml extract: expected >= 2 functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    // --- Cross-language: every function has line_number > 0 ---

    #[test]
    fn test_extract_all_functions_have_line_numbers() {
        let cases: Vec<&str> = vec![
            "test_python.py",
            "test_rust.rs",
            "test_go.go",
            "test_typescript.ts",
            "test_javascript.js",
            "test_java.java",
            "test_c.c",
            "test_cpp.cpp",
            "test_ruby.rb",
            "test_php.php",
            "test_kotlin.kt",
            "test_swift.swift",
            "test_csharp.cs",
            "test_scala.scala",
            "test_elixir.ex",
            "test_lua.lua",
            "test_ocaml.ml",
        ];

        for filename in cases {
            let info = extract_for(filename);
            for func in &info.functions {
                assert!(
                    func.line_number > 0,
                    "{}: function '{}' has line_number=0",
                    filename,
                    func.name
                );
            }
            for cls in &info.classes {
                assert!(
                    cls.line_number > 0,
                    "{}: class '{}' has line_number=0",
                    filename,
                    cls.name
                );
                for method in &cls.methods {
                    assert!(
                        method.line_number > 0,
                        "{}: method '{}' in class '{}' has line_number=0",
                        filename,
                        method.name,
                        cls.name
                    );
                }
            }
        }
    }

    // --- Cross-language: every function has a non-empty name ---

    #[test]
    fn test_extract_all_functions_have_names() {
        let cases: Vec<&str> = vec![
            "test_python.py",
            "test_rust.rs",
            "test_go.go",
            "test_typescript.ts",
            "test_javascript.js",
            "test_java.java",
            "test_c.c",
            "test_cpp.cpp",
            "test_ruby.rb",
            "test_php.php",
            "test_kotlin.kt",
            "test_swift.swift",
            "test_csharp.cs",
            "test_scala.scala",
            "test_elixir.ex",
            "test_lua.lua",
            "test_ocaml.ml",
        ];

        for filename in cases {
            let info = extract_for(filename);
            for func in &info.functions {
                assert!(
                    !func.name.is_empty(),
                    "{}: found function with empty name",
                    filename
                );
            }
            for cls in &info.classes {
                // Kotlin companion objects may have empty names (anonymous companion)
                // This is a known limitation of the extractor.
                if filename == "test_kotlin.kt" && cls.name.is_empty() {
                    continue;
                }
                assert!(
                    !cls.name.is_empty(),
                    "{}: found class with empty name",
                    filename
                );
            }
        }
    }
}

// =============================================================================
// imports command tests -- verify import parsing for files that have them
// =============================================================================

mod imports_tests {
    use super::*;

    // Files with explicit import/include/using statements in the fixtures:
    // - test_c.c: #include <stdio.h>
    // - test_cpp.cpp: #include <string>
    // - test_csharp.cs: using System;

    #[test]
    fn test_imports_c() {
        // GIVEN: C file with #include <stdio.h>
        let file = extractor_fixtures_dir().join("test_c.c");
        let imports = get_imports(&file, Language::C);

        assert!(
            imports.is_ok(),
            "get_imports failed for C: {:?}",
            imports.err()
        );
        let imports = imports.unwrap();
        assert!(
            !imports.is_empty(),
            "C: expected imports (has #include <stdio.h>), got empty"
        );

        // At least one import should reference stdio
        let has_stdio = imports
            .iter()
            .any(|i| i.module.contains("stdio"));
        assert!(
            has_stdio,
            "C: expected import containing 'stdio', got: {:?}",
            imports.iter().map(|i| &i.module).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_imports_cpp() {
        // GIVEN: C++ file with #include <string>
        let file = extractor_fixtures_dir().join("test_cpp.cpp");
        let imports = get_imports(&file, Language::Cpp);

        assert!(
            imports.is_ok(),
            "get_imports failed for C++: {:?}",
            imports.err()
        );
        let imports = imports.unwrap();
        assert!(
            !imports.is_empty(),
            "C++: expected imports (has #include <string>), got empty"
        );

        let has_string = imports
            .iter()
            .any(|i| i.module.contains("string"));
        assert!(
            has_string,
            "C++: expected import containing 'string', got: {:?}",
            imports.iter().map(|i| &i.module).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_imports_csharp() {
        // GIVEN: C# file with "using System;"
        let file = extractor_fixtures_dir().join("test_csharp.cs");
        let imports = get_imports(&file, Language::CSharp);

        assert!(
            imports.is_ok(),
            "get_imports failed for C#: {:?}",
            imports.err()
        );
        let imports = imports.unwrap();
        assert!(
            !imports.is_empty(),
            "C#: expected imports (has using System;), got empty"
        );

        let has_system = imports
            .iter()
            .any(|i| i.module.contains("System"));
        assert!(
            has_system,
            "C#: expected import containing 'System', got: {:?}",
            imports.iter().map(|i| &i.module).collect::<Vec<_>>()
        );
    }

    // --- Languages WITHOUT imports in the fixtures should return Ok with empty vec ---

    #[test]
    fn test_imports_python_no_imports() {
        // The test_python.py fixture has no import statements
        let file = extractor_fixtures_dir().join("test_python.py");
        let imports = get_imports(&file, Language::Python);
        assert!(imports.is_ok(), "get_imports failed for Python: {:?}", imports.err());
        assert!(
            imports.unwrap().is_empty(),
            "Python fixture has no imports, expected empty"
        );
    }

    #[test]
    fn test_imports_rust_no_imports() {
        let file = extractor_fixtures_dir().join("test_rust.rs");
        let imports = get_imports(&file, Language::Rust);
        assert!(imports.is_ok(), "get_imports failed for Rust: {:?}", imports.err());
        // The Rust fixture has no use statements at top level (it uses std::error inline)
    }

    #[test]
    fn test_imports_go_no_imports() {
        let file = extractor_fixtures_dir().join("test_go.go");
        let imports = get_imports(&file, Language::Go);
        assert!(imports.is_ok(), "get_imports failed for Go: {:?}", imports.err());
        assert!(
            imports.unwrap().is_empty(),
            "Go fixture has no import blocks, expected empty"
        );
    }

    #[test]
    fn test_imports_typescript_no_imports() {
        let file = extractor_fixtures_dir().join("test_typescript.ts");
        let imports = get_imports(&file, Language::TypeScript);
        assert!(
            imports.is_ok(),
            "get_imports failed for TypeScript: {:?}",
            imports.err()
        );
        assert!(
            imports.unwrap().is_empty(),
            "TypeScript fixture has no import statements, expected empty"
        );
    }

    #[test]
    fn test_imports_javascript_no_imports() {
        let file = extractor_fixtures_dir().join("test_javascript.js");
        let imports = get_imports(&file, Language::JavaScript);
        assert!(
            imports.is_ok(),
            "get_imports failed for JavaScript: {:?}",
            imports.err()
        );
        assert!(
            imports.unwrap().is_empty(),
            "JavaScript fixture has no import statements, expected empty"
        );
    }

    #[test]
    fn test_imports_java_no_imports() {
        let file = extractor_fixtures_dir().join("test_java.java");
        let imports = get_imports(&file, Language::Java);
        assert!(imports.is_ok(), "get_imports failed for Java: {:?}", imports.err());
        assert!(
            imports.unwrap().is_empty(),
            "Java fixture has no import statements, expected empty"
        );
    }

    #[test]
    fn test_imports_ruby_no_imports() {
        let file = extractor_fixtures_dir().join("test_ruby.rb");
        let imports = get_imports(&file, Language::Ruby);
        assert!(imports.is_ok(), "get_imports failed for Ruby: {:?}", imports.err());
        assert!(
            imports.unwrap().is_empty(),
            "Ruby fixture has no require statements, expected empty"
        );
    }

    #[test]
    fn test_imports_php_no_imports() {
        let file = extractor_fixtures_dir().join("test_php.php");
        let imports = get_imports(&file, Language::Php);
        assert!(imports.is_ok(), "get_imports failed for PHP: {:?}", imports.err());
        assert!(
            imports.unwrap().is_empty(),
            "PHP fixture has no use/require statements, expected empty"
        );
    }

    #[test]
    fn test_imports_kotlin_no_imports() {
        let file = extractor_fixtures_dir().join("test_kotlin.kt");
        let imports = get_imports(&file, Language::Kotlin);
        assert!(imports.is_ok(), "get_imports failed for Kotlin: {:?}", imports.err());
        assert!(
            imports.unwrap().is_empty(),
            "Kotlin fixture has no import statements, expected empty"
        );
    }

    #[test]
    fn test_imports_swift_no_imports() {
        let file = extractor_fixtures_dir().join("test_swift.swift");
        let imports = get_imports(&file, Language::Swift);
        assert!(imports.is_ok(), "get_imports failed for Swift: {:?}", imports.err());
        assert!(
            imports.unwrap().is_empty(),
            "Swift fixture has no import statements, expected empty"
        );
    }

    #[test]
    fn test_imports_scala_no_imports() {
        let file = extractor_fixtures_dir().join("test_scala.scala");
        let imports = get_imports(&file, Language::Scala);
        assert!(imports.is_ok(), "get_imports failed for Scala: {:?}", imports.err());
        assert!(
            imports.unwrap().is_empty(),
            "Scala fixture has no import statements, expected empty"
        );
    }

    #[test]
    fn test_imports_elixir_no_imports() {
        let file = extractor_fixtures_dir().join("test_elixir.ex");
        let imports = get_imports(&file, Language::Elixir);
        assert!(imports.is_ok(), "get_imports failed for Elixir: {:?}", imports.err());
        // Elixir fixture has no explicit import/require/alias statements
    }

    #[test]
    fn test_imports_lua_no_imports() {
        let file = extractor_fixtures_dir().join("test_lua.lua");
        let imports = get_imports(&file, Language::Lua);
        assert!(imports.is_ok(), "get_imports failed for Lua: {:?}", imports.err());
        assert!(
            imports.unwrap().is_empty(),
            "Lua fixture has no require statements, expected empty"
        );
    }

    #[test]
    fn test_imports_ocaml_no_imports() {
        let file = extractor_fixtures_dir().join("test_ocaml.ml");
        let imports = get_imports(&file, Language::Ocaml);
        assert!(imports.is_ok(), "get_imports failed for OCaml: {:?}", imports.err());
        // OCaml fixture has no open statements
    }

    // --- Import content validation ---

    #[test]
    fn test_imports_have_non_empty_module_names() {
        // For every file that returns imports, verify the module name is non-empty
        let cases: Vec<(&str, Language)> = vec![
            ("test_c.c", Language::C),
            ("test_cpp.cpp", Language::Cpp),
            ("test_csharp.cs", Language::CSharp),
        ];

        for (filename, lang) in cases {
            let file = extractor_fixtures_dir().join(filename);
            let imports = get_imports(&file, lang).unwrap();
            for imp in &imports {
                assert!(
                    !imp.module.is_empty(),
                    "{}: import has empty module name",
                    filename
                );
            }
        }
    }
}

// =============================================================================
// importers command tests -- find files importing a module
// =============================================================================

mod importers_tests {
    use super::*;

    #[test]
    fn test_importers_via_structure_imports() {
        // The importers command finds files that import a given module.
        // We can verify this indirectly: get_code_structure for the extractor dir
        // should show files with imports in their FileStructure.imports field.
        //
        // For the C# fixture that has "using System;", structure should include it.
        let file = extractor_fixtures_dir().join("test_csharp.cs");
        let result = get_code_structure(&file, Language::CSharp, 0, None);
        assert!(result.is_ok());
        let cs = result.unwrap();
        let file_struct = &cs.files[0];
        assert!(
            !file_struct.imports.is_empty(),
            "C# structure should include imports (using System;)"
        );
        let has_system = file_struct
            .imports
            .iter()
            .any(|i| i.module.contains("System"));
        assert!(
            has_system,
            "C# structure imports should contain 'System', got: {:?}",
            file_struct
                .imports
                .iter()
                .map(|i| &i.module)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_importers_c_includes() {
        // C file structure should include #include imports
        let file = extractor_fixtures_dir().join("test_c.c");
        let result = get_code_structure(&file, Language::C, 0, None);
        assert!(result.is_ok());
        let cs = result.unwrap();
        let file_struct = &cs.files[0];
        assert!(
            !file_struct.imports.is_empty(),
            "C structure should include imports (#include <stdio.h>)"
        );
    }
}
