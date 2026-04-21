//! Trigger keyword extraction for API surface entries.
//!
//! Triggers map user intent (e.g., "parse JSON") to the correct API
//! (e.g., `json.loads`). They are generated from:
//! 1. Verb splitting: `get_user_by_id` -> ["get", "user", "id"]
//! 2. Docstring nouns: first line of docstring -> key nouns
//!
//! These triggers enable intent-based retrieval without requiring
//! the user to know the exact function name.

/// Extract trigger keywords from a method/function name by splitting on underscores
/// and filtering out common noise words.
///
/// # Examples
/// ```
/// use tldr_core::contracts::triggers::extract_name_triggers;
/// let triggers = extract_name_triggers("get_user_by_id");
/// assert!(triggers.contains(&"get".to_string()));
/// assert!(triggers.contains(&"user".to_string()));
/// assert!(triggers.contains(&"id".to_string()));
/// ```
pub fn extract_name_triggers(name: &str) -> Vec<String> {
    let parts = split_identifier(name);
    parts
        .into_iter()
        .filter(|w| !is_noise_word(w))
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_lowercase())
        .collect()
}

/// Extract keyword nouns from the first line of a docstring.
///
/// Strips common verbs and articles, returning content words that
/// can serve as triggers for intent-based retrieval.
pub fn extract_docstring_triggers(docstring: &str) -> Vec<String> {
    let first_line = docstring
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches('.');

    first_line
        .split_whitespace()
        .filter_map(|word| {
            let cleaned = word
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
                .to_lowercase();
            if cleaned.len() >= 2 && !is_noise_word(&cleaned) && !is_common_verb(&cleaned) {
                Some(cleaned)
            } else {
                None
            }
        })
        .collect()
}

/// Combine name and docstring triggers, deduplicating.
pub fn extract_triggers(name: &str, docstring: Option<&str>) -> Vec<String> {
    let mut triggers = extract_name_triggers(name);

    if let Some(doc) = docstring {
        for trigger in extract_docstring_triggers(doc) {
            if !triggers.contains(&trigger) {
                triggers.push(trigger);
            }
        }
    }

    triggers
}

/// Split an identifier into component words.
///
/// Handles both `snake_case` and `camelCase` conventions:
/// - `get_user_by_id` -> ["get", "user", "by", "id"]
/// - `getUserById` -> ["get", "User", "By", "Id"] -> lowercased later
/// - `__init__` -> ["init"]
fn split_identifier(name: &str) -> Vec<String> {
    // Strip leading/trailing underscores (dunder methods)
    let stripped = name.trim_matches('_');
    if stripped.is_empty() {
        return Vec::new();
    }

    // First split on underscores (snake_case)
    let underscore_parts: Vec<&str> = stripped.split('_').filter(|s| !s.is_empty()).collect();

    let mut result = Vec::new();
    for part in underscore_parts {
        // Then split on camelCase boundaries
        let camel_parts = split_camel_case(part);
        result.extend(camel_parts);
    }

    result
}

/// Split a camelCase string into individual words.
///
/// "getUserById" -> ["get", "User", "By", "Id"]
/// "HTTPClient" -> ["HTTP", "Client"]
fn split_camel_case(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = s.chars().collect();

    for i in 0..chars.len() {
        let c = chars[i];
        if c.is_uppercase() && !current.is_empty() {
            // Check if this is the start of a new word
            let prev_lower = i > 0 && chars[i - 1].is_lowercase();
            let next_lower = i + 1 < chars.len() && chars[i + 1].is_lowercase();

            if prev_lower || (next_lower && current.len() > 1) {
                parts.push(current);
                current = String::new();
            }
        }
        current.push(c);
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

/// Check if a word is a noise word that should be filtered from triggers.
fn is_noise_word(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "a" | "an"
            | "the"
            | "of"
            | "in"
            | "to"
            | "for"
            | "by"
            | "is"
            | "it"
            | "or"
            | "and"
            | "on"
            | "at"
            | "if"
            | "as"
            | "be"
            | "no"
            | "do"
            | "up"
            | "so"
            | "my"
            | "self"
            | "cls"
            | "this"
    )
}

/// Check if a word is a common verb that appears in docstrings
/// but is too generic to be a useful trigger on its own.
fn is_common_verb(word: &str) -> bool {
    matches!(
        word,
        "return" | "returns" | "get" | "set" | "has" | "does" | "can" | "will" | "should" | "may"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_identifier_snake_case() {
        assert_eq!(
            split_identifier("get_user_by_id"),
            vec!["get", "user", "by", "id"]
        );
    }

    #[test]
    fn test_split_identifier_camel_case() {
        let parts = split_identifier("getUserById");
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "get");
    }

    #[test]
    fn test_split_identifier_dunder() {
        assert_eq!(split_identifier("__init__"), vec!["init"]);
    }

    #[test]
    fn test_split_identifier_single() {
        assert_eq!(split_identifier("loads"), vec!["loads"]);
    }

    #[test]
    fn test_split_identifier_empty_underscores() {
        assert_eq!(split_identifier("___"), Vec::<String>::new());
    }

    #[test]
    fn test_split_camel_case_acronym() {
        let parts = split_camel_case("HTTPClient");
        assert_eq!(parts, vec!["HTTP", "Client"]);
    }

    #[test]
    fn test_extract_name_triggers() {
        let triggers = extract_name_triggers("get_user_by_id");
        assert!(triggers.contains(&"get".to_string()));
        assert!(triggers.contains(&"user".to_string()));
        assert!(triggers.contains(&"id".to_string()));
        // "by" is a noise word and should be filtered
        assert!(!triggers.contains(&"by".to_string()));
    }

    #[test]
    fn test_extract_name_triggers_filters_short() {
        let triggers = extract_name_triggers("a_b_test");
        // "a" and "b" are < 2 chars, should be filtered
        assert!(!triggers.contains(&"a".to_string()));
        assert!(triggers.contains(&"test".to_string()));
    }

    #[test]
    fn test_extract_docstring_triggers() {
        let triggers = extract_docstring_triggers("Deserialize s to a Python object.");
        assert!(triggers.contains(&"deserialize".to_string()));
        assert!(triggers.contains(&"python".to_string()));
        assert!(triggers.contains(&"object".to_string()));
        // "a" and "to" are noise words
        assert!(!triggers.contains(&"a".to_string()));
        assert!(!triggers.contains(&"to".to_string()));
    }

    #[test]
    fn test_extract_docstring_triggers_empty() {
        let triggers = extract_docstring_triggers("");
        assert!(triggers.is_empty());
    }

    #[test]
    fn test_extract_triggers_combined() {
        let triggers = extract_triggers("load_json", Some("Parse a JSON string into an object."));
        assert!(triggers.contains(&"load".to_string()));
        assert!(triggers.contains(&"json".to_string()));
        assert!(triggers.contains(&"parse".to_string()));
        assert!(triggers.contains(&"string".to_string()));
        assert!(triggers.contains(&"object".to_string()));
    }

    #[test]
    fn test_extract_triggers_deduplication() {
        let triggers = extract_triggers("parse_json", Some("Parse JSON data."));
        // "parse" should appear only once even though it's in both name and docstring
        let parse_count = triggers.iter().filter(|t| *t == "parse").count();
        assert_eq!(parse_count, 1);
    }

    #[test]
    fn test_extract_triggers_no_docstring() {
        let triggers = extract_triggers("read_file", None);
        assert!(triggers.contains(&"read".to_string()));
        assert!(triggers.contains(&"file".to_string()));
    }
}
