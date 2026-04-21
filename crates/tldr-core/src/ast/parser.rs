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
use std::sync::{Arc, Mutex};

use tree_sitter::{Language, Parser, Tree};

use crate::error::TldrError;
use crate::types::Language as TldrLanguage;
use crate::TldrResult;

/// Maximum file size to parse (5MB) - M6 mitigation
pub const MAX_PARSE_SIZE: usize = 5 * 1024 * 1024;

/// Thread-safe parser pool that reuses parsers per language
pub struct ParserPool {
    parsers: Mutex<HashMap<TldrLanguage, Parser>>,
}

impl ParserPool {
    /// Create a new parser pool
    pub fn new() -> Self {
        Self {
            parsers: Mutex::new(HashMap::new()),
        }
    }

    /// Get tree-sitter Language for a TLDR language
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

    /// Parse source code and return a tree
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
        // Check file size - M6 mitigation
        if source.len() > MAX_PARSE_SIZE {
            return Err(TldrError::ParseError {
                file: std::path::PathBuf::from("<source>"),
                line: None,
                message: format!(
                    "File too large: {} bytes (max {})",
                    source.len(),
                    MAX_PARSE_SIZE
                ),
            });
        }

        let ts_lang = Self::get_ts_language(lang)
            .ok_or_else(|| TldrError::UnsupportedLanguage(lang.to_string()))?;

        // Get or create parser for this language
        let mut parsers = self.parsers.lock().unwrap();
        let parser = parsers.entry(lang).or_insert_with(|| {
            let mut p = Parser::new();
            p.set_language(&ts_lang).expect("Error loading grammar");
            p
        });

        // Ensure parser has the correct language set
        // Note: We reset on each call since the parser pool may have been used for different languages
        parser
            .set_language(&ts_lang)
            .map_err(|e| TldrError::ParseError {
                file: std::path::PathBuf::from("<source>"),
                line: None,
                message: format!("Failed to set language: {}", e),
            })?;

        // Parse the source
        parser
            .parse(source, None)
            .ok_or_else(|| TldrError::ParseError {
                file: std::path::PathBuf::from("<source>"),
                line: None,
                message: "Parsing returned None".to_string(),
            })
    }

    /// Parse a file from disk
    ///
    /// Handles encoding with UTF-8 lossy fallback (M2 mitigation).
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

        // Parse the source
        let tree = self.parse(&source, lang).map_err(|e| {
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

/// Parse source code using the global parser pool
pub fn parse(source: &str, lang: TldrLanguage) -> TldrResult<Tree> {
    PARSER_POOL.parse(source, lang)
}

/// Parse a file using the global parser pool
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
}
