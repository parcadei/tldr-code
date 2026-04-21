//! Flow analysis tools: cfg, dfg, slice, complexity, pdg
//!
//! These tools provide control flow and data flow analysis for functions.

use crate::protocol::ToolsCallResult;
use serde_json::Value;

use super::{get_optional_int, get_optional_string, get_required_string, to_path};

/// Handle tldr_cfg tool call
pub fn handle_cfg(args: Value) -> ToolsCallResult {
    let file = match get_required_string(&args, "file") {
        Ok(f) => f,
        Err(e) => return ToolsCallResult::error(e),
    };

    let function = match get_required_string(&args, "function") {
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

    match tldr_core::get_cfg_context(file_path.to_str().unwrap_or(""), &function, lang) {
        Ok(cfg) => match serde_json::to_string_pretty(&cfg) {
            Ok(json) => ToolsCallResult::text(json),
            Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
        },
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_complexity tool call
pub fn handle_complexity(args: Value) -> ToolsCallResult {
    let file = match get_required_string(&args, "file") {
        Ok(f) => f,
        Err(e) => return ToolsCallResult::error(e),
    };

    let function = match get_required_string(&args, "function") {
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

    match tldr_core::calculate_complexity(file_path.to_str().unwrap_or(""), &function, lang) {
        Ok(metrics) => match serde_json::to_string_pretty(&metrics) {
            Ok(json) => ToolsCallResult::text(json),
            Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
        },
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_dfg tool call
pub fn handle_dfg(args: Value) -> ToolsCallResult {
    let file = match get_required_string(&args, "file") {
        Ok(f) => f,
        Err(e) => return ToolsCallResult::error(e),
    };

    let function = match get_required_string(&args, "function") {
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

    match tldr_core::get_dfg_context(file_path.to_str().unwrap_or(""), &function, lang) {
        Ok(dfg) => match serde_json::to_string_pretty(&dfg) {
            Ok(json) => ToolsCallResult::text(json),
            Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
        },
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_slice tool call
pub fn handle_slice(args: Value) -> ToolsCallResult {
    let file = match get_required_string(&args, "file") {
        Ok(f) => f,
        Err(e) => return ToolsCallResult::error(e),
    };

    let function = match get_required_string(&args, "function") {
        Ok(f) => f,
        Err(e) => return ToolsCallResult::error(e),
    };

    let line = match get_optional_int(&args, "line") {
        Some(l) => l as u32,
        None => return ToolsCallResult::error("Missing required argument: line"),
    };

    let direction_str =
        get_optional_string(&args, "direction").unwrap_or_else(|| "backward".to_string());
    let variable = get_optional_string(&args, "variable");

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

    let direction = match direction_str.parse::<tldr_core::SliceDirection>() {
        Ok(d) => d,
        Err(e) => return ToolsCallResult::error(e),
    };

    match tldr_core::get_slice(
        file_path.to_str().unwrap_or(""),
        &function,
        line,
        direction,
        variable.as_deref(),
        lang,
    ) {
        Ok(lines) => {
            let mut sorted_lines: Vec<_> = lines.into_iter().collect();
            sorted_lines.sort();
            match serde_json::to_string_pretty(&serde_json::json!({
                "function": function,
                "line": line,
                "direction": direction_str,
                "slice_lines": sorted_lines
            })) {
                Ok(json) => ToolsCallResult::text(json),
                Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
            }
        }
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_pdg tool call
pub fn handle_pdg(args: Value) -> ToolsCallResult {
    let file = match get_required_string(&args, "file") {
        Ok(f) => f,
        Err(e) => return ToolsCallResult::error(e),
    };

    let function = match get_required_string(&args, "function") {
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

    match tldr_core::get_pdg_context(file_path.to_str().unwrap_or(""), &function, lang) {
        Ok(pdg) => match serde_json::to_string_pretty(&pdg) {
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
    fn test_handle_cfg_missing_args() {
        let result = handle_cfg(json!({}));
        assert!(result.is_error == Some(true));
    }

    #[test]
    fn test_handle_slice_missing_line() {
        let result = handle_slice(json!({"file": "test.py", "function": "foo"}));
        assert!(result.is_error == Some(true));
        assert!(result.content[0].text.contains("line"));
    }

    #[test]
    fn test_handle_dfg_file_not_found() {
        let result = handle_dfg(json!({
            "file": "/nonexistent/file.py",
            "function": "foo"
        }));
        assert!(result.is_error == Some(true));
        assert!(result.content[0].text.contains("File not found"));
    }
}
