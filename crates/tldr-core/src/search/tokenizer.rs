//! Code-aware tokenizer for BM25 search
//!
//! Tokenizes code identifiers by splitting:
//! - camelCase: `processData` -> `["process", "data"]`
//! - snake_case: `process_data` -> `["process", "data"]`
//! - PascalCase: `ProcessData` -> `["process", "data"]`
//! - SCREAMING_CASE: `PROCESS_DATA` -> `["process", "data"]`
//!
//! # Mitigation M11
//! This tokenizer must match the Python implementation exactly to ensure
//! BM25 search results have >= 80% overlap.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Code-aware tokenizer for BM25 search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokenizer {
    /// Stopwords to filter out
    stopwords: HashSet<String>,
    /// Minimum token length
    min_length: usize,
}

impl Default for Tokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Tokenizer {
    /// Create a new tokenizer with default settings
    pub fn new() -> Self {
        Self {
            stopwords: Self::default_stopwords(),
            min_length: 2,
        }
    }

    /// Create a tokenizer with custom stopwords
    pub fn with_stopwords(stopwords: HashSet<String>) -> Self {
        Self {
            stopwords,
            min_length: 2,
        }
    }

    /// Default stopwords for code search
    ///
    /// Includes common programming keywords that don't add semantic value
    fn default_stopwords() -> HashSet<String> {
        [
            // Common programming keywords
            "def",
            "class",
            "function",
            "fn",
            "func",
            "pub",
            "private",
            "public",
            "static",
            "const",
            "let",
            "var",
            "mut",
            "if",
            "else",
            "elif",
            "then",
            "for",
            "while",
            "do",
            "loop",
            "break",
            "continue",
            "return",
            "yield",
            "try",
            "catch",
            "except",
            "finally",
            "throw",
            "raise",
            "import",
            "from",
            "export",
            "module",
            "package",
            "use",
            "require",
            "include",
            "with",
            "as",
            "in",
            "is",
            "not",
            "and",
            "or",
            "true",
            "false",
            "null",
            "none",
            "nil",
            "self",
            "this",
            "super",
            "new",
            "delete",
            "sizeof",
            "typeof",
            "instanceof",
            // Common short words
            "a",
            "an",
            "the",
            "to",
            "of",
            "on",
            "at",
            "by",
            "it",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    /// Tokenize a string into searchable tokens
    ///
    /// # Example
    /// ```
    /// use tldr_core::search::tokenizer::Tokenizer;
    ///
    /// let tokenizer = Tokenizer::new();
    /// let tokens = tokenizer.tokenize("processUserData_v2");
    /// assert!(tokens.contains(&"process".to_string()));
    /// assert!(tokens.contains(&"user".to_string()));
    /// assert!(tokens.contains(&"data".to_string()));
    /// ```
    pub fn tokenize(&self, text: &str) -> Vec<String> {
        let mut tokens = Vec::new();

        // First, split on whitespace and punctuation
        for word in Self::split_on_delimiters(text) {
            // Then split camelCase and snake_case
            for token in self.split_identifier(&word) {
                let lower = token.to_lowercase();

                // Filter by length and stopwords
                if lower.len() >= self.min_length && !self.stopwords.contains(&lower) {
                    tokens.push(lower);
                }
            }
        }

        tokens
    }

    /// Tokenize and return unique tokens
    pub fn tokenize_unique(&self, text: &str) -> HashSet<String> {
        self.tokenize(text).into_iter().collect()
    }

    /// Split text on whitespace and punctuation delimiters
    fn split_on_delimiters(text: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut current = String::new();

        for ch in text.chars() {
            if ch.is_alphanumeric() || ch == '_' {
                current.push(ch);
            } else if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
        }

        if !current.is_empty() {
            result.push(current);
        }

        result
    }

    /// Split a single identifier by camelCase and snake_case
    ///
    /// Examples:
    /// - `processData` -> `["process", "Data"]`
    /// - `process_data` -> `["process", "data"]`
    /// - `ProcessUserData` -> `["Process", "User", "Data"]`
    /// - `HTTPRequest` -> `["HTTP", "Request"]`
    fn split_identifier(&self, word: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current = String::new();
        let mut prev_was_upper = false;
        let mut prev_was_underscore = false;

        let chars: Vec<char> = word.chars().collect();

        for (i, &ch) in chars.iter().enumerate() {
            if ch == '_' {
                // Underscore is a delimiter in snake_case
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                prev_was_underscore = true;
                prev_was_upper = false;
                continue;
            }

            let is_upper = ch.is_uppercase();
            let next_is_lower = chars.get(i + 1).map(|c| c.is_lowercase()).unwrap_or(false);

            // Start new token on:
            // 1. Transition from lower to upper (camelCase boundary)
            // 2. Transition from upper to upper+lower (HTTPRequest -> HTTP, Request)
            // 3. After underscore
            let should_split = !current.is_empty()
                && (prev_was_underscore
                    || !prev_was_upper && is_upper
                    || is_upper && next_is_lower);

            if should_split {
                tokens.push(std::mem::take(&mut current));
            }

            current.push(ch);
            prev_was_upper = is_upper;
            prev_was_underscore = false;
        }

        if !current.is_empty() {
            tokens.push(current);
        }

        tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_camel_case() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("processData");
        assert!(tokens.contains(&"process".to_string()));
        assert!(tokens.contains(&"data".to_string()));
    }

    #[test]
    fn test_tokenize_snake_case() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("process_data");
        assert!(tokens.contains(&"process".to_string()));
        assert!(tokens.contains(&"data".to_string()));
    }

    #[test]
    fn test_tokenize_pascal_case() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("ProcessUserData");
        assert!(tokens.contains(&"process".to_string()));
        assert!(tokens.contains(&"user".to_string()));
        assert!(tokens.contains(&"data".to_string()));
    }

    #[test]
    fn test_tokenize_http_abbreviation() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("HTTPRequest");
        assert!(tokens.contains(&"http".to_string()));
        assert!(tokens.contains(&"request".to_string()));
    }

    #[test]
    fn test_tokenize_mixed() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("processUserData_v2");
        assert!(tokens.contains(&"process".to_string()));
        assert!(tokens.contains(&"user".to_string()));
        assert!(tokens.contains(&"data".to_string()));
        assert!(tokens.contains(&"v2".to_string()));
    }

    #[test]
    fn test_tokenize_filters_stopwords() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("def processData");
        // "def" should be filtered as stopword
        assert!(!tokens.contains(&"def".to_string()));
        assert!(tokens.contains(&"process".to_string()));
        assert!(tokens.contains(&"data".to_string()));
    }

    #[test]
    fn test_tokenize_case_insensitive() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("PROCESS_DATA");
        assert!(tokens.contains(&"process".to_string()));
        assert!(tokens.contains(&"data".to_string()));
    }

    #[test]
    fn test_split_identifier_simple() {
        let tokenizer = Tokenizer::new();
        let parts = tokenizer.split_identifier("processData");
        assert_eq!(parts, vec!["process", "Data"]);
    }

    #[test]
    fn test_split_identifier_snake() {
        let tokenizer = Tokenizer::new();
        let parts = tokenizer.split_identifier("process_data");
        assert_eq!(parts, vec!["process", "data"]);
    }
}
