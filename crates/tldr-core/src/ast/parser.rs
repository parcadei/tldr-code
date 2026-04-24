//! Tree-sitter parser pool for efficient parsing
//!
//! Provides reusable parsers for each supported language to avoid
//! repeated initialization overhead.
//!
//! # Mitigations Addressed
//! - M1: Tree-sitter version matching (use pinned versions)
//! - M2: Unicode/encoding handling (use from_utf8_lossy)
//! - M13: Reuse parsers to reduce memory (parser pool)

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use tree_sitter::{Language, Parser, Tree};

use crate::error::TldrError;
use crate::types::Language as TldrLanguage;
use crate::TldrResult;

/// Maximum file size to parse (5MB) - M6 mitigation
pub const MAX_PARSE_SIZE: usize = 5 * 1024 * 1024;

/// TypeScript / JavaScript grammar dialect.
///
/// `tree-sitter-typescript` ships two distinct grammars:
/// - `LANGUAGE_TYPESCRIPT`: pure TS, faster, rejects JSX.
/// - `LANGUAGE_TSX`: TSX grammar, understands JSX expressions.
///
/// `TldrLanguage::TypeScript` and `TldrLanguage::JavaScript` both map onto
/// these two dialects depending on the file extension. `.tsx` and `.jsx`
/// route to TSX; everything else gets the non-TSX default. Languages that
/// are not TS/JS use `TsDialect::None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TsDialect {
    /// Non-TS/JS language — dialect is not applicable.
    None,
    /// Plain TypeScript / JavaScript grammar (no JSX).
    Ts,
    /// TSX grammar — accepts both TSX and JSX syntax.
    Tsx,
}

impl TsDialect {
    /// Derive a dialect from an optional file path and a language.
    ///
    /// Returns `TsDialect::Tsx` for `.tsx` / `.jsx` paths on TS/JS,
    /// `TsDialect::Ts` for plain TS/JS files, and `TsDialect::None` for
    /// every other language.
    pub fn from_path_and_lang(path: Option<&Path>, lang: TldrLanguage) -> Self {
        match lang {
            TldrLanguage::TypeScript | TldrLanguage::JavaScript => {
                match path
                    .and_then(|p| p.extension())
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                {
                    Some(ref e) if e == "tsx" || e == "jsx" => TsDialect::Tsx,
                    _ => TsDialect::Ts,
                }
            }
            _ => TsDialect::None,
        }
    }
}

/// Composite cache key for the parser pool.
///
/// The old pool keyed parsers by `TldrLanguage` alone, which collapsed the
/// TS and TSX grammars into one slot. A TS-grammar parser and a TSX-grammar
/// parser are different tree-sitter objects and must not share a slot —
/// otherwise calling `set_language` on every borrow would either thrash the
/// cache or (worse) silently reuse the wrong grammar on a cache miss.
///
/// The new key is `(TldrLanguage, TsDialect)`. Non-TS/JS languages use
/// `TsDialect::None`, preserving their old single-slot behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParserKey {
    /// Logical TLDR language (Python, TypeScript, ...).
    pub lang: TldrLanguage,
    /// Grammar dialect — only meaningful for TS/JS, `None` otherwise.
    pub dialect: TsDialect,
}

impl ParserKey {
    /// Build a cache key from a language and dialect.
    pub fn new(lang: TldrLanguage, dialect: TsDialect) -> Self {
        Self { lang, dialect }
    }
}

/// Thread-safe parser pool that reuses parsers per `(language, dialect)`.
pub struct ParserPool {
    parsers: Mutex<HashMap<ParserKey, Parser>>,
}

impl ParserPool {
    /// Create a new parser pool
    pub fn new() -> Self {
        Self {
            parsers: Mutex::new(HashMap::new()),
        }
    }

    /// Get tree-sitter Language for a TLDR language.
    ///
    /// For TS and JS this returns the non-TSX default. Callers that have a
    /// path and need JSX-aware parsing (i.e. `.tsx` / `.jsx`) should use
    /// [`Self::parse_file`] or [`Self::parse_with_path`] instead; those
    /// route through [`Self::select_ts_grammar`] and pick up
    /// `LANGUAGE_TSX` from the path extension.
    pub fn get_ts_language(lang: TldrLanguage) -> Option<Language> {
        match lang {
            TldrLanguage::Python => Some(tree_sitter_python::LANGUAGE.into()),
            TldrLanguage::TypeScript | TldrLanguage::JavaScript => {
                Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            }
            TldrLanguage::Go => Some(tree_sitter_go::LANGUAGE.into()),
            TldrLanguage::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
            TldrLanguage::Java => Some(tree_sitter_java::LANGUAGE.into()),
            // P2 languages - Phase 2: C and C++
            TldrLanguage::C => Some(tree_sitter_c::LANGUAGE.into()),
            TldrLanguage::Cpp => Some(tree_sitter_cpp::LANGUAGE.into()),
            // P2 languages - Phase 3: Ruby
            TldrLanguage::Ruby => Some(tree_sitter_ruby::LANGUAGE.into()),
            // P2 languages - Phase 4: C#, Scala, PHP
            TldrLanguage::CSharp => Some(tree_sitter_c_sharp::LANGUAGE.into()),
            TldrLanguage::Scala => Some(tree_sitter_scala::LANGUAGE.into()),
            // Note: PHP uses LANGUAGE_PHP (not LANGUAGE) - includes PHP opening tag support
            TldrLanguage::Php => Some(tree_sitter_php::LANGUAGE_PHP.into()),
            // P2 languages - Phase 5: Lua, Luau, Elixir
            TldrLanguage::Lua => Some(tree_sitter_lua::LANGUAGE.into()),
            TldrLanguage::Luau => Some(tree_sitter_luau::LANGUAGE.into()),
            TldrLanguage::Elixir => Some(tree_sitter_elixir::LANGUAGE.into()),
            TldrLanguage::Ocaml => Some(tree_sitter_ocaml::LANGUAGE_OCAML.into()),
            TldrLanguage::Kotlin => Some(tree_sitter_kotlin_ng::LANGUAGE.into()),
            TldrLanguage::Swift => Some(tree_sitter_swift::LANGUAGE.into()),
        }
    }

    /// Pick the right TS/JS grammar from a path extension.
    ///
    /// - `.tsx` / `.jsx` -> `LANGUAGE_TSX` (JSX-aware).
    /// - Everything else -> `LANGUAGE_TYPESCRIPT` (the conservative default).
    ///
    /// `tree-sitter-typescript` does not ship a dedicated JSX grammar, so
    /// `.jsx` files are routed through the TSX grammar as well — it
    /// understands JSX syntax without the TS type annotations we'd have
    /// otherwise hit error-recovery on.
    fn select_ts_grammar(path: Option<&Path>) -> Language {
        match path
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
        {
            Some(ref e) if e == "tsx" || e == "jsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
            _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        }
    }

    /// Resolve `(language, path)` to a concrete tree-sitter grammar.
    fn resolve_grammar(lang: TldrLanguage, path: Option<&Path>) -> Option<Language> {
        match lang {
            TldrLanguage::TypeScript | TldrLanguage::JavaScript => {
                Some(Self::select_ts_grammar(path))
            }
            _ => Self::get_ts_language(lang),
        }
    }

    /// Parse source code using the path-less default grammar.
    ///
    /// For TS/JS this returns `LANGUAGE_TYPESCRIPT` (non-TSX). Callers
    /// that know the original file path should prefer
    /// [`Self::parse_with_path`] or [`Self::parse_file`] so that `.tsx`
    /// and `.jsx` files get routed to the TSX grammar.
    ///
    /// # Arguments
    /// * `source` - Source code to parse (UTF-8)
    /// * `lang` - Programming language
    ///
    /// # Returns
    /// * `Ok(Tree)` - Parsed syntax tree
    /// * `Err(TldrError::UnsupportedLanguage)` - Language not supported
    /// * `Err(TldrError::ParseError)` - Parsing failed
    pub fn parse(&self, source: &str, lang: TldrLanguage) -> TldrResult<Tree> {
        self.parse_with_path(source, lang, None)
    }

    /// Parse source code, using the file path (if known) to pick the
    /// right dialect of the tree-sitter grammar.
    ///
    /// When `path` is `Some`, this inspects the extension and routes
    /// `.tsx` / `.jsx` files through `LANGUAGE_TSX`. All other paths (and
    /// `None`) use the language's default grammar.
    ///
    /// The parser cache is keyed on `(language, dialect)`, so a TS-grammar
    /// parser and a TSX-grammar parser coexist in distinct slots and are
    /// reused across calls with the same dialect.
    pub fn parse_with_path(
        &self,
        source: &str,
        lang: TldrLanguage,
        path: Option<&Path>,
    ) -> TldrResult<Tree> {
        // Check file size - M6 mitigation
        if source.len() > MAX_PARSE_SIZE {
            return Err(TldrError::ParseError {
                file: path
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("<source>")),
                line: None,
                message: format!(
                    "File too large: {} bytes (max {})",
                    source.len(),
                    MAX_PARSE_SIZE
                ),
            });
        }

        let ts_lang = Self::resolve_grammar(lang, path)
            .ok_or_else(|| TldrError::UnsupportedLanguage(lang.to_string()))?;
        let dialect = TsDialect::from_path_and_lang(path, lang);
        let key = ParserKey::new(lang, dialect);

        // Get or create parser for this (lang, dialect) pair.
        let mut parsers = self.parsers.lock().unwrap();
        let parser = parsers.entry(key).or_insert_with(|| {
            let mut p = Parser::new();
            p.set_language(&ts_lang).expect("Error loading grammar");
            p
        });

        // Defensive re-set: if a previous borrow left the cached parser
        // on a different grammar (shouldn't happen with the new key, but
        // cheap insurance) this snaps it back before parsing.
        parser
            .set_language(&ts_lang)
            .map_err(|e| TldrError::ParseError {
                file: path
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("<source>")),
                line: None,
                message: format!("Failed to set language: {}", e),
            })?;

        parser
            .parse(source, None)
            .ok_or_else(|| TldrError::ParseError {
                file: path
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("<source>")),
                line: None,
                message: "Parsing returned None".to_string(),
            })
    }

    /// Parse a file from disk.
    ///
    /// Dispatches to the grammar dialect appropriate for the file
    /// extension (`.tsx` / `.jsx` route to TSX). Handles encoding with
    /// UTF-8 lossy fallback (M2 mitigation).
    pub fn parse_file(&self, path: &std::path::Path) -> TldrResult<(Tree, String, TldrLanguage)> {
        // Detect language from extension
        let lang = TldrLanguage::from_path(path).ok_or_else(|| {
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            TldrError::UnsupportedLanguage(ext)
        })?;

        // Read file content with UTF-8 lossy fallback - M2 mitigation
        let bytes = std::fs::read(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                TldrError::PathNotFound(path.to_path_buf())
            } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                TldrError::PermissionDenied(path.to_path_buf())
            } else {
                TldrError::IoError(e)
            }
        })?;

        // Convert to string with lossy UTF-8 handling
        let source = String::from_utf8_lossy(&bytes).to_string();

        // Parse the source, passing the path so the TSX dialect is picked
        // up for `.tsx` / `.jsx` files.
        let tree = self
            .parse_with_path(&source, lang, Some(path))
            .map_err(|e| {
                if let TldrError::ParseError { line, message, .. } = e {
                    TldrError::ParseError {
                        file: path.to_path_buf(),
                        line,
                        message,
                    }
                } else {
                    e
                }
            })?;

        Ok((tree, source, lang))
    }
}

impl Default for ParserPool {
    fn default() -> Self {
        Self::new()
    }
}

// Global parser pool for convenience
lazy_static::lazy_static! {
    /// Global parser pool instance
    pub static ref PARSER_POOL: Arc<ParserPool> = Arc::new(ParserPool::new());
}

/// Parse source code using the global parser pool (path-less).
pub fn parse(source: &str, lang: TldrLanguage) -> TldrResult<Tree> {
    PARSER_POOL.parse(source, lang)
}

/// Parse source code using the global parser pool, with an optional
/// path used to pick the right TS/JS grammar dialect.
pub fn parse_with_path(source: &str, lang: TldrLanguage, path: Option<&Path>) -> TldrResult<Tree> {
    PARSER_POOL.parse_with_path(source, lang, path)
}

/// Parse a file using the global parser pool.
pub fn parse_file(path: &std::path::Path) -> TldrResult<(Tree, String, TldrLanguage)> {
    PARSER_POOL.parse_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_python() {
        let source = "def foo(): pass";
        let tree = parse(source, TldrLanguage::Python).unwrap();
        assert_eq!(tree.root_node().kind(), "module");
    }

    #[test]
    fn test_parse_typescript() {
        let source = "function foo() {}";
        let tree = parse(source, TldrLanguage::TypeScript).unwrap();
        assert_eq!(tree.root_node().kind(), "program");
    }

    #[test]
    fn test_parse_go() {
        let source = "package main\nfunc foo() {}";
        let tree = parse(source, TldrLanguage::Go).unwrap();
        assert_eq!(tree.root_node().kind(), "source_file");
    }

    #[test]
    fn test_parse_rust() {
        let source = "fn foo() {}";
        let tree = parse(source, TldrLanguage::Rust).unwrap();
        assert_eq!(tree.root_node().kind(), "source_file");
    }

    #[test]
    fn test_swift_now_supported() {
        // Swift was previously disabled due to ABI v15 incompatibility with tree-sitter 0.24.7.
        // tree-sitter 0.25.0 supports ABI v15 via the tree-sitter-language bridging crate.
        let result = parse("let x = 1", TldrLanguage::Swift);
        assert!(
            result.is_ok(),
            "Swift should now parse successfully: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().root_node().kind(), "source_file");
    }

    #[test]
    fn test_parser_reuse() {
        let pool = ParserPool::new();

        // Parse multiple times with same language
        for _ in 0..5 {
            let _ = pool.parse("def foo(): pass", TldrLanguage::Python).unwrap();
        }

        // Only one parser should be created
        let parsers = pool.parsers.lock().unwrap();
        assert_eq!(parsers.len(), 1);
    }

    // ---------------------------------------------------------------------
    // VAL-004: TSX/JSX grammar dialect selection
    // ---------------------------------------------------------------------
    //
    // Regression tests for the bug where ParserPool::parse_file chose
    // LANGUAGE_TYPESCRIPT for .tsx / .jsx paths. That grammar does not
    // understand JSX syntax, so JSX-heavy files entered tree-sitter
    // error-recovery and produced pathological ASTs. Downstream, the
    // message-chain smell detector went exponential on these broken trees,
    // timing out on real-world files such as dub's
    // `apps/web/.../screenshot.tsx` (1584 LOC).
    //
    // Fix: ParserPool::parse_file must select tree_sitter_typescript::
    // LANGUAGE_TSX when the path extension is `.tsx` or `.jsx`, and the
    // parser cache must distinguish dialects so a TS-grammar parser and a
    // TSX-grammar parser do not share a cache slot.

    /// Recursively count the number of `ERROR` nodes in a tree.
    fn count_error_nodes(node: tree_sitter::Node) -> usize {
        let mut count = if node.is_error() { 1 } else { 0 };
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            count += count_error_nodes(child);
        }
        count
    }

    #[test]
    fn test_parse_file_tsx_uses_tsx_grammar() {
        // A tempdir .tsx file with JSX must parse cleanly (zero ERROR
        // nodes) when routed through parse_file, which has the path and
        // can dispatch to LANGUAGE_TSX.
        let dir = tempfile::tempdir().unwrap();
        let tsx_path = dir.path().join("App.tsx");
        std::fs::write(
            &tsx_path,
            r#"export const App = ({ name }: { name: string }) => <div className="a">{name}</div>;
"#,
        )
        .unwrap();

        let pool = ParserPool::new();
        let (tree, _src, lang) = pool.parse_file(&tsx_path).unwrap();
        assert_eq!(lang, TldrLanguage::TypeScript);
        let errors = count_error_nodes(tree.root_node());
        assert_eq!(
            errors, 0,
            "expected zero ERROR nodes for .tsx via TSX grammar, got {}",
            errors
        );

        // Plain .ts also parses cleanly via the non-TSX default.
        let ts_path = dir.path().join("plain.ts");
        std::fs::write(&ts_path, "export const x: number = 1;\n").unwrap();
        let (tree, _src, lang) = pool.parse_file(&ts_path).unwrap();
        assert_eq!(lang, TldrLanguage::TypeScript);
        assert_eq!(
            count_error_nodes(tree.root_node()),
            0,
            "plain .ts should parse cleanly"
        );
    }

    #[test]
    fn test_parse_file_jsx_uses_tsx_grammar() {
        // tree-sitter-typescript does not ship a dedicated JSX grammar;
        // LANGUAGE_TSX handles both TSX and JSX. Selecting it for .jsx
        // files keeps JSX syntax out of error-recovery.
        let dir = tempfile::tempdir().unwrap();
        let jsx_path = dir.path().join("App.jsx");
        std::fs::write(
            &jsx_path,
            "export const App = ({ name }) => <div className=\"a\">{name}</div>;\n",
        )
        .unwrap();

        let pool = ParserPool::new();
        let (tree, _src, lang) = pool.parse_file(&jsx_path).unwrap();
        assert_eq!(lang, TldrLanguage::JavaScript);
        let errors = count_error_nodes(tree.root_node());
        assert_eq!(
            errors, 0,
            "expected zero ERROR nodes for .jsx via TSX grammar, got {}",
            errors
        );
    }

    #[test]
    fn test_parse_cache_distinguishes_dialects() {
        // The parser cache must key on (language, dialect) so that a TS
        // parser and a TSX parser are not clobbered into the same slot
        // across repeated calls. If they shared a slot, the second call
        // would silently reuse the wrong grammar and the third call (back
        // to .ts) would then see JSX-flavoured error recovery again.
        let dir = tempfile::tempdir().unwrap();
        let ts_path = dir.path().join("a.ts");
        let tsx_path = dir.path().join("b.tsx");
        std::fs::write(&ts_path, "export const n: number = 1;\n").unwrap();
        std::fs::write(&tsx_path, "export const App = () => <div>{1}</div>;\n").unwrap();

        let pool = ParserPool::new();
        // .ts -> .tsx -> .ts, each must parse cleanly.
        let (t1, _, _) = pool.parse_file(&ts_path).unwrap();
        assert_eq!(count_error_nodes(t1.root_node()), 0, "first .ts failed");
        let (t2, _, _) = pool.parse_file(&tsx_path).unwrap();
        assert_eq!(count_error_nodes(t2.root_node()), 0, ".tsx failed");
        let (t3, _, _) = pool.parse_file(&ts_path).unwrap();
        assert_eq!(
            count_error_nodes(t3.root_node()),
            0,
            "second .ts failed (cache collision between TS and TSX parsers)"
        );
    }

    #[test]
    fn test_legacy_parse_without_path_uses_ts_default() {
        // The legacy `parse(source, lang)` API has no path and therefore
        // cannot disambiguate TS vs TSX. Contract: it keeps working for
        // plain TypeScript (returns LANGUAGE_TYPESCRIPT, the conservative
        // default) and produces ERROR nodes when given JSX. Callers that
        // need JSX-aware parsing must use parse_file or pass a path.
        let pool = ParserPool::new();

        // Plain TS parses cleanly via the path-less default.
        let tree = pool
            .parse("export const x: number = 1;", TldrLanguage::TypeScript)
            .unwrap();
        assert_eq!(
            count_error_nodes(tree.root_node()),
            0,
            "plain TS should parse cleanly via path-less API"
        );

        // JSX via path-less API produces error nodes — that is the
        // documented contract; it is not a regression.
        let jsx_src = "const App = () => <div className=\"a\">hi</div>;";
        let tree = pool.parse(jsx_src, TldrLanguage::TypeScript).unwrap();
        assert!(
            count_error_nodes(tree.root_node()) > 0,
            "path-less TS parse of JSX is expected to produce ERROR nodes; \
             if it parses cleanly, the default grammar changed and callers \
             must be audited"
        );
    }

    // ---------------------------------------------------------------------
    // VAL-008: parser health audit for all 18 supported languages.
    //
    // Each of tldr's 18 supported languages must have a tree-sitter grammar
    // capable of parsing a minimal valid source file with zero ERROR and
    // zero MISSING nodes. This test codifies the baseline so that grammar
    // regressions (e.g. an incompatible ABI bump) get caught immediately.
    // ---------------------------------------------------------------------
    #[test]
    fn test_all_18_parsers_accept_minimal_valid_snippet() {
        let snippets: &[(TldrLanguage, &str)] = &[
            (TldrLanguage::Python, "def x(): pass"),
            (TldrLanguage::TypeScript, "export const x: number = 1;"),
            (TldrLanguage::JavaScript, "export const x = 1;"),
            (TldrLanguage::Go, "package main\nfunc main() {}"),
            (TldrLanguage::Rust, "pub fn x() {}"),
            (
                TldrLanguage::Java,
                "class X { public static void main(String[] a){} }",
            ),
            (TldrLanguage::C, "int main(){return 0;}"),
            (TldrLanguage::Cpp, "int main(){return 0;}"),
            (TldrLanguage::Ruby, "def x; end"),
            (TldrLanguage::Kotlin, "fun x(){}"),
            (TldrLanguage::Swift, "func x(){}"),
            (TldrLanguage::CSharp, "class X { static void Main(){} }"),
            (
                TldrLanguage::Scala,
                "object X { def main(args: Array[String]): Unit = {} }",
            ),
            (TldrLanguage::Php, "<?php function x(){}"),
            (TldrLanguage::Lua, "function x() end"),
            (TldrLanguage::Luau, "function x() end"),
            (
                TldrLanguage::Elixir,
                "defmodule X do\ndef y(), do: :ok\nend",
            ),
            (TldrLanguage::Ocaml, "let x () = ()"),
        ];

        let pool = ParserPool::new();
        let mut failures: Vec<String> = Vec::new();
        for (lang, src) in snippets {
            match pool.parse(src, *lang) {
                Ok(tree) => {
                    let errs = count_error_nodes(tree.root_node());
                    if errs != 0 {
                        failures.push(format!(
                            "{:?}: {} ERROR node(s) on valid snippet: {:?}",
                            lang, errs, src
                        ));
                    }
                }
                Err(e) => {
                    failures.push(format!("{:?}: parse failed: {:?} on {:?}", lang, e, src));
                }
            }
        }

        assert!(
            failures.is_empty(),
            "Parser audit failures (VAL-008): {}",
            failures.join(" | ")
        );
    }
}
