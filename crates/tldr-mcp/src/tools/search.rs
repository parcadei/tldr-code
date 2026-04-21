//! Search tools: search, bm25, semantic
//!
//! These tools provide various search capabilities over codebases.

use crate::protocol::ToolsCallResult;
use serde_json::Value;
use std::collections::HashSet;

use super::{
    get_optional_int, get_optional_string, get_optional_string_array, get_required_string, to_path,
};

/// Handle tldr_search tool call (regex search)
pub fn handle_search(args: Value) -> ToolsCallResult {
    let pattern = match get_required_string(&args, "pattern") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let extensions = get_optional_string_array(&args, "extensions");
    let context_lines = get_optional_int(&args, "context_lines").unwrap_or(0) as usize;
    let max_results = get_optional_int(&args, "max_results").unwrap_or(100) as usize;
    let max_files = get_optional_int(&args, "max_files").unwrap_or(1000) as usize;

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    // Convert extensions to HashSet if provided
    let ext_set: Option<HashSet<String>> = extensions.map(|exts| {
        exts.into_iter()
            .map(|e| {
                if e.starts_with('.') {
                    e
                } else {
                    format!(".{}", e)
                }
            })
            .collect()
    });

    match tldr_core::search(
        &pattern,
        &path,
        ext_set.as_ref(),
        context_lines,
        max_results,
        max_files,
        None,
    ) {
        Ok(matches) => {
            match serde_json::to_string_pretty(&serde_json::json!({
                "pattern": pattern,
                "total_matches": matches.len(),
                "matches": matches
            })) {
                Ok(json) => ToolsCallResult::text(json),
                Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
            }
        }
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_bm25 tool call (BM25 keyword search)
pub fn handle_bm25(args: Value) -> ToolsCallResult {
    let query = match get_required_string(&args, "query") {
        Ok(q) => q,
        Err(e) => return ToolsCallResult::error(e),
    };

    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let top_k = get_optional_int(&args, "top_k").unwrap_or(10) as usize;

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    // Auto-detect language from path or use provided
    let language = get_optional_string(&args, "language");
    let lang = if let Some(l) = language {
        match l.parse::<tldr_core::Language>() {
            Ok(lang) => lang,
            Err(e) => return ToolsCallResult::error(e),
        }
    } else {
        // Default to Python for directory searches
        tldr_core::Language::Python
    };

    // Build BM25 index from project
    let index = match tldr_core::Bm25Index::from_project(&path, lang) {
        Ok(idx) => idx,
        Err(e) => return ToolsCallResult::error(format!("Error building index: {}", e)),
    };

    let results = index.search(&query, top_k);

    match serde_json::to_string_pretty(&serde_json::json!({
        "query": query,
        "total_results": results.len(),
        "results": results
    })) {
        Ok(json) => ToolsCallResult::text(json),
        Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
    }
}

/// Handle tldr_semantic tool call (hybrid search with embeddings)
pub fn handle_semantic(args: Value) -> ToolsCallResult {
    let query = match get_required_string(&args, "query") {
        Ok(q) => q,
        Err(e) => return ToolsCallResult::error(e),
    };

    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let top_k = get_optional_int(&args, "top_k").unwrap_or(10) as usize;

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    // Auto-detect language from path or use provided
    let language = get_optional_string(&args, "language");
    let lang = if let Some(l) = language {
        match l.parse::<tldr_core::Language>() {
            Ok(lang) => lang,
            Err(e) => return ToolsCallResult::error(e),
        }
    } else {
        // Default to Python for directory searches
        tldr_core::Language::Python
    };

    // Hybrid search (falls back to BM25-only if embedding service unavailable)
    // M8: Graceful degradation when embedding service is not available
    match tldr_core::hybrid_search(&query, &path, lang, top_k, 60.0, None) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => ToolsCallResult::text(json),
            Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
        },
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_handle_search_missing_pattern() {
        let result = handle_search(json!({"path": "."}));
        assert!(result.is_error == Some(true));
        assert!(result.content[0].text.contains("Missing required argument"));
    }

    #[test]
    fn test_handle_search_missing_path() {
        let result = handle_search(json!({"pattern": "test"}));
        assert!(result.is_error == Some(true));
    }

    #[test]
    fn test_handle_bm25_path_not_found() {
        let result = handle_bm25(json!({
            "query": "test",
            "path": "/nonexistent/path"
        }));
        assert!(result.is_error == Some(true));
        assert!(result.content[0].text.contains("Path not found"));
    }
}
