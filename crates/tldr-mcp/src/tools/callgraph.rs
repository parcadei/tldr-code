//! Call graph tools: calls, impact, dead, importers, arch
//!
//! These tools provide cross-file analysis of function calls and dependencies.

use crate::protocol::ToolsCallResult;
use serde_json::Value;

use super::{
    get_optional_int, get_optional_string, get_optional_string_array, get_required_string, to_path,
};

/// Handle tldr_calls tool call
pub fn handle_calls(args: Value) -> ToolsCallResult {
    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let language = match get_required_string(&args, "language") {
        Ok(l) => l,
        Err(e) => return ToolsCallResult::error(e),
    };

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    let lang = match language.parse::<tldr_core::Language>() {
        Ok(l) => l,
        Err(e) => return ToolsCallResult::error(e),
    };

    match tldr_core::build_project_call_graph(&path, lang, None, true) {
        Ok(call_graph) => {
            // Serialize the call graph edges
            let edges: Vec<_> = call_graph.edges().collect();
            match serde_json::to_string_pretty(&serde_json::json!({
                "edge_count": call_graph.edge_count(),
                "edges": edges
            })) {
                Ok(json) => ToolsCallResult::text(json),
                Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
            }
        }
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_impact tool call
pub fn handle_impact(args: Value) -> ToolsCallResult {
    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let function = match get_required_string(&args, "function") {
        Ok(f) => f,
        Err(e) => return ToolsCallResult::error(e),
    };

    let language = match get_required_string(&args, "language") {
        Ok(l) => l,
        Err(e) => return ToolsCallResult::error(e),
    };

    let depth = get_optional_int(&args, "depth").unwrap_or(3) as usize;
    let file_filter = get_optional_string(&args, "file");

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    let lang = match language.parse::<tldr_core::Language>() {
        Ok(l) => l,
        Err(e) => return ToolsCallResult::error(e),
    };

    // First build the call graph
    let call_graph = match tldr_core::build_project_call_graph(&path, lang, None, true) {
        Ok(cg) => cg,
        Err(e) => return ToolsCallResult::error(format!("Error building call graph: {}", e)),
    };

    let file_path = file_filter.map(|f| to_path(&f));

    match tldr_core::impact_analysis(&call_graph, &function, depth, file_path.as_deref()) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => ToolsCallResult::text(json),
            Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
        },
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_dead tool call
pub fn handle_dead(args: Value) -> ToolsCallResult {
    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let language = match get_required_string(&args, "language") {
        Ok(l) => l,
        Err(e) => return ToolsCallResult::error(e),
    };

    let entry_points = get_optional_string_array(&args, "entry_points");

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    let lang = match language.parse::<tldr_core::Language>() {
        Ok(l) => l,
        Err(e) => return ToolsCallResult::error(e),
    };

    // Build call graph first
    let call_graph = match tldr_core::build_project_call_graph(&path, lang, None, true) {
        Ok(cg) => cg,
        Err(e) => return ToolsCallResult::error(format!("Error building call graph: {}", e)),
    };

    // Get all functions from structure
    let all_functions = match tldr_core::get_code_structure(&path, lang, 0, None) {
        Ok(structure) => {
            let mut funcs = Vec::new();
            for file in structure.files {
                for func_name in file.functions {
                    funcs.push(tldr_core::FunctionRef::new(file.path.clone(), func_name));
                }
            }
            funcs
        }
        Err(e) => return ToolsCallResult::error(format!("Error getting structure: {}", e)),
    };

    let entry_refs: Option<Vec<String>> = entry_points;

    match tldr_core::dead_code_analysis(&call_graph, &all_functions, entry_refs.as_deref()) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => ToolsCallResult::text(json),
            Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
        },
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_importers tool call
pub fn handle_importers(args: Value) -> ToolsCallResult {
    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let module = match get_required_string(&args, "module") {
        Ok(m) => m,
        Err(e) => return ToolsCallResult::error(e),
    };

    let language = match get_required_string(&args, "language") {
        Ok(l) => l,
        Err(e) => return ToolsCallResult::error(e),
    };

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    let lang = match language.parse::<tldr_core::Language>() {
        Ok(l) => l,
        Err(e) => return ToolsCallResult::error(e),
    };

    match tldr_core::find_importers(&path, &module, lang) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => ToolsCallResult::text(json),
            Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
        },
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_arch tool call
pub fn handle_arch(args: Value) -> ToolsCallResult {
    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let language = match get_required_string(&args, "language") {
        Ok(l) => l,
        Err(e) => return ToolsCallResult::error(e),
    };

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    let lang = match language.parse::<tldr_core::Language>() {
        Ok(l) => l,
        Err(e) => return ToolsCallResult::error(e),
    };

    // Build call graph first
    let call_graph = match tldr_core::build_project_call_graph(&path, lang, None, true) {
        Ok(cg) => cg,
        Err(e) => return ToolsCallResult::error(format!("Error building call graph: {}", e)),
    };

    match tldr_core::architecture_analysis(&call_graph) {
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
    fn test_handle_calls_missing_args() {
        let result = handle_calls(json!({}));
        assert!(result.is_error == Some(true));
    }

    #[test]
    fn test_handle_impact_missing_function() {
        let result = handle_impact(json!({"path": ".", "language": "python"}));
        assert!(result.is_error == Some(true));
        assert!(result.content[0].text.contains("Missing required argument"));
    }

    #[test]
    fn test_handle_dead_missing_language() {
        let result = handle_dead(json!({"path": "."}));
        assert!(result.is_error == Some(true));
    }
}
