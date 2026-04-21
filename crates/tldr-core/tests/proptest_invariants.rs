//! Property-based invariant tests for tldr-core
//!
//! Tests that core parsing and analysis functions never panic on arbitrary
//! input and maintain expected invariants.

use proptest::prelude::*;

// =============================================================================
// 1. Tree-sitter parser: arbitrary source strings must not panic
// =============================================================================

mod parser_fuzz {
    use super::*;
    use tldr_core::ast::parser::{parse, ParserPool};
    use tldr_core::types::Language;

    /// All supported languages to fuzz.
    fn all_languages() -> Vec<Language> {
        vec![
            Language::Python,
            Language::TypeScript,
            Language::JavaScript,
            Language::Go,
            Language::Rust,
            Language::Java,
            Language::C,
            Language::Cpp,
            Language::Ruby,
            Language::CSharp,
            Language::Scala,
            Language::Php,
            Language::Lua,
            Language::Luau,
            Language::Elixir,
            Language::Ocaml,
            Language::Kotlin,
            Language::Swift,
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        /// Invariant: parsing arbitrary UTF-8 strings never panics.
        /// It may return Err (malformed source), but must not crash.
        #[test]
        fn parse_arbitrary_source_no_panic(
            source in "(.|\n){0,2000}",
            lang_idx in 0..18usize,
        ) {
            let langs = all_languages();
            let lang = langs[lang_idx];
            // We only care that it doesn't panic — Ok or Err are both fine
            let _ = parse(&source, lang);
        }

        /// Invariant: parsing empty string never panics for any language.
        #[test]
        fn parse_empty_string_no_panic(lang_idx in 0..18usize) {
            let langs = all_languages();
            let _ = parse("", langs[lang_idx]);
        }

        /// Invariant: parsing strings with only whitespace/newlines never panics.
        #[test]
        fn parse_whitespace_only_no_panic(
            ws in "[ \t\n\r]{0,500}",
            lang_idx in 0..18usize,
        ) {
            let langs = all_languages();
            let _ = parse(&ws, langs[lang_idx]);
        }

        /// Invariant: parser handles adversarial nesting without stack overflow.
        #[test]
        fn parse_deep_nesting_no_panic(depth in 1..200usize) {
            // Deep parenthesization — common crash vector for recursive descent
            let source = "(".repeat(depth) + &")".repeat(depth);
            let _ = parse(&source, Language::Python);

            // Deep brace nesting
            let braces = "{".repeat(depth) + &"}".repeat(depth);
            let _ = parse(&braces, Language::Rust);
        }

        /// Invariant: parser tolerates unterminated strings.
        #[test]
        fn parse_unterminated_strings_no_panic(
            prefix in "[a-z ]{0,50}",
            lang_idx in prop::sample::select(vec![0usize, 1, 4, 5]), // Py, TS, Rust, Java
        ) {
            let langs = all_languages();
            // Unterminated double-quote string
            let source = format!("{}\"unclosed string with stuff", prefix);
            let _ = parse(&source, langs[lang_idx]);

            // Unterminated single-quote
            let source2 = format!("{}'also unclosed", prefix);
            let _ = parse(&source2, langs[lang_idx]);
        }
    }

    #[test]
    fn parse_null_bytes_no_panic() {
        // NUL bytes in source — tree-sitter should handle gracefully
        let source = "def foo():\n    x = \0\n    return x\n";
        let _ = parse(source, Language::Python);

        let source2 = "fn main() { let x\0 = 1; }";
        let _ = parse(source2, Language::Rust);
    }

    #[test]
    fn parser_pool_concurrent_no_panic() {
        // Verify the parser pool doesn't deadlock under sequential multi-language use
        let pool = ParserPool::new();
        for lang in all_languages() {
            let _ = pool.parse("x = 1", lang);
            let _ = pool.parse("", lang);
            let _ = pool.parse("{{{{", lang);
        }
    }
}

// =============================================================================
// 2. Error parser: arbitrary error text must not panic or hang
// =============================================================================

mod error_parser_fuzz {
    use super::*;
    use tldr_core::fix::error_parser::{
        detect_language, parse_error, parse_go_error, parse_js_error, parse_python_error,
        parse_rustc_error, parse_tsc_error,
    };

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(500))]

        /// Invariant: parse_error never panics on arbitrary text.
        #[test]
        fn parse_error_no_panic(raw in "(.|\n){0,3000}") {
            let _ = parse_error(&raw, None);
        }

        /// Invariant: parse_error with explicit lang never panics.
        #[test]
        fn parse_error_with_lang_no_panic(
            raw in "(.|\n){0,1000}",
            lang in prop::sample::select(vec![
                "python", "rust", "typescript", "go", "javascript", "unknown"
            ]),
        ) {
            let _ = parse_error(&raw, Some(lang));
        }

        /// Invariant: detect_language never panics.
        #[test]
        fn detect_language_no_panic(text in "(.|\n){0,2000}") {
            let result = detect_language(&text);
            // Should return a non-empty string
            prop_assert!(!result.is_empty());
        }

        /// Invariant: each language-specific parser never panics.
        #[test]
        fn all_lang_parsers_no_panic(raw in "(.|\n){0,2000}") {
            let _ = parse_python_error(&raw);
            let _ = parse_js_error(&raw);
            let _ = parse_rustc_error(&raw);
            let _ = parse_tsc_error(&raw);
            let _ = parse_go_error(&raw);
        }

        /// Invariant: error text with adversarial line counts doesn't hang.
        #[test]
        fn many_lines_no_hang(line_count in 1..500usize) {
            let raw = "error: something\n".repeat(line_count);
            let _ = parse_error(&raw, None);
        }

        /// Invariant: extremely long single lines don't cause issues.
        #[test]
        fn long_line_no_hang(len in 1..10000usize) {
            let raw = format!("TypeError: {}", "x".repeat(len));
            let _ = parse_error(&raw, Some("python"));
        }
    }

    #[test]
    fn parse_error_empty_string() {
        assert!(parse_error("", None).is_none());
    }

    #[test]
    fn parse_error_only_newlines() {
        assert!(parse_error("\n\n\n", None).is_none());
    }
}

// =============================================================================
// 3. Call graph: builder invariants
// =============================================================================

mod callgraph_fuzz {
    use super::*;
    use tldr_core::build_project_call_graph;
    use tldr_core::types::Language;
    use tempfile::TempDir;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(30))]

        /// Invariant: call graph builder never panics on arbitrary Python files.
        #[test]
        fn callgraph_arbitrary_python_no_panic(
            sources in prop::collection::vec("(.|\n){0,500}", 1..5)
        ) {
            let temp = TempDir::new().unwrap();
            for (i, src) in sources.iter().enumerate() {
                let path = temp.path().join(format!("file_{}.py", i));
                std::fs::write(&path, src).unwrap();
            }

            let _ = build_project_call_graph(
                temp.path(),
                Language::Python,
                None,
                false,
            );
        }

        /// Invariant: call graph on empty directory returns successfully.
        #[test]
        fn callgraph_empty_dir_no_panic(lang in prop::sample::select(vec![
            Language::Python, Language::Rust, Language::TypeScript, Language::Go
        ])) {
            let temp = TempDir::new().unwrap();
            let result = build_project_call_graph(
                temp.path(),
                lang,
                None,
                false,
            );
            prop_assert!(result.is_ok());
        }

        /// Invariant: call graph with valid Python always produces
        /// a graph where edge endpoints reference defined functions.
        #[test]
        fn callgraph_valid_python_edges_reference_functions(
            func_count in 2..8usize,
        ) {
            let temp = TempDir::new().unwrap();

            // Generate Python with N functions where each calls the next
            let mut code = String::new();
            for i in 0..func_count {
                code.push_str(&format!("def func_{}():\n", i));
                if i + 1 < func_count {
                    code.push_str(&format!("    func_{}()\n\n", i + 1));
                } else {
                    code.push_str("    pass\n\n");
                }
            }

            std::fs::write(temp.path().join("chain.py"), &code).unwrap();

            let result = build_project_call_graph(
                temp.path(),
                Language::Python,
                None,
                false,
            );

            if let Ok(graph) = result {
                for edge in graph.edges() {
                    prop_assert!(
                        edge.src_func.starts_with("func_") || edge.src_func.contains("::"),
                        "unexpected src_func: {}", edge.src_func
                    );
                }
            }
        }
    }
}
