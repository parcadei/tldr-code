//! AST tools: tree, structure, extract, imports
//!
//! These tools provide navigation and structural analysis of codebases.

use crate::protocol::ToolsCallResult;
use serde_json::Value;

use super::{
    get_optional_bool, get_optional_int, get_optional_string, get_optional_string_array,
    get_required_string, to_path,
};

/// Handle tldr_tree tool call
pub fn handle_tree(args: Value) -> ToolsCallResult {
    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let extensions = get_optional_string_array(&args, "extensions");
    let exclude_hidden = get_optional_bool(&args, "exclude_hidden").unwrap_or(true);

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    // Convert extensions to HashSet if provided
    let ext_set = extensions.map(|exts| {
        exts.into_iter()
            .map(|e| {
                if e.starts_with('.') {
                    e
                } else {
                    format!(".{}", e)
                }
            })
            .collect::<std::collections::HashSet<String>>()
    });

    match tldr_core::get_file_tree(&path, ext_set.as_ref(), exclude_hidden, None) {
        Ok(tree) => match serde_json::to_string_pretty(&tree) {
            Ok(json) => ToolsCallResult::text(json),
            Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
        },
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_structure tool call
pub fn handle_structure(args: Value) -> ToolsCallResult {
    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let language = match get_required_string(&args, "language") {
        Ok(l) => l,
        Err(e) => return ToolsCallResult::error(e),
    };

    let max_results = get_optional_int(&args, "max_results").unwrap_or(0) as usize;

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    let lang = match language.parse::<tldr_core::Language>() {
        Ok(l) => l,
        Err(e) => return ToolsCallResult::error(e),
    };

    match tldr_core::get_code_structure(&path, lang, max_results, None) {
        Ok(structure) => match serde_json::to_string_pretty(&structure) {
            Ok(json) => ToolsCallResult::text(json),
            Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
        },
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_extract tool call
pub fn handle_extract(args: Value) -> ToolsCallResult {
    let file = match get_required_string(&args, "file") {
        Ok(f) => f,
        Err(e) => return ToolsCallResult::error(e),
    };

    let base_path = get_optional_string(&args, "base_path");

    let file_path = to_path(&file);
    if !file_path.exists() {
        return ToolsCallResult::error(format!("File not found: {}", file_path.display()));
    }

    let base = base_path.map(|p| to_path(&p));

    match tldr_core::extract_file(&file_path, base.as_deref()) {
        Ok(module_info) => match serde_json::to_string_pretty(&module_info) {
            Ok(json) => ToolsCallResult::text(json),
            Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
        },
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_imports tool call
pub fn handle_imports(args: Value) -> ToolsCallResult {
    let file = match get_required_string(&args, "file") {
        Ok(f) => f,
        Err(e) => return ToolsCallResult::error(e),
    };

    let file_path = to_path(&file);
    if !file_path.exists() {
        return ToolsCallResult::error(format!("File not found: {}", file_path.display()));
    }

    // Auto-detect language from extension if not provided
    let language = get_optional_string(&args, "language");
    let lang = if let Some(l) = language {
        match l.parse::<tldr_core::Language>() {
            Ok(lang) => lang,
            Err(e) => return ToolsCallResult::error(e),
        }
    } else {
        match tldr_core::Language::from_path(&file_path) {
            Some(l) => l,
            None => {
                return ToolsCallResult::error(format!(
                    "Could not detect language for file: {}. Please specify language explicitly.",
                    file_path.display()
                ))
            }
        }
    };

    match tldr_core::get_imports(&file_path, lang) {
        Ok(imports) => match serde_json::to_string_pretty(&imports) {
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
    fn test_handle_tree_missing_path() {
        let result = handle_tree(json!({}));
        assert!(result.is_error == Some(true));
        assert!(result.content[0].text.contains("Missing required argument"));
    }

    #[test]
    fn test_handle_tree_path_not_found() {
        let result = handle_tree(json!({"path": "/nonexistent/path"}));
        assert!(result.is_error == Some(true));
        assert!(result.content[0].text.contains("Path not found"));
    }

    #[test]
    fn test_handle_structure_missing_language() {
        let result = handle_structure(json!({"path": "."}));
        assert!(result.is_error == Some(true));
        assert!(result.content[0].text.contains("Missing required argument"));
    }
}
