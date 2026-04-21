use std::path::Path;

use tree_sitter::Node;

use super::super::language_profile::{
    node_text, LanguageNodeMap, LanguageProfile, LanguageSemantics, SignalAction,
};
use super::super::signals::{detect_naming_case, PatternSignals};

/// Semantic extractor for Elixir.
pub struct ElixirSemantics;

impl LanguageSemantics for ElixirSemantics {
    fn process_call(
        &self,
        call_id: &str,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        let call_text = node_text(node, source);

        match call_id {
            "defmodule" => {
                if let Some(name) = extract_identifier_arg(&call_text, "defmodule") {
                    let case = detect_naming_case(&name);
                    signals
                        .naming
                        .class_names
                        .push((name, case, file_path.display().to_string()));
                }
            }
            "def" | "defp" => {
                if let Some(name) = extract_identifier_arg(&call_text, call_id) {
                    let normalized = name.trim_end_matches("do").trim().to_string();
                    let function_name = normalized
                        .split('(')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !function_name.is_empty() {
                        let case = detect_naming_case(&function_name);
                        signals.naming.function_names.push((
                            function_name.clone(),
                            case,
                            file_path.display().to_string(),
                        ));
                        if function_name.starts_with("test") {
                            signals.test_idioms.test_function_count += 1;
                        }
                    }
                }
            }
            "import" | "alias" | "use" | "require" => {
                if let Some(module) = extract_identifier_arg(&call_text, call_id) {
                    signals
                        .import_patterns
                        .absolute_imports
                        .push((module, file_path.display().to_string()));
                }
            }
            "test" => {
                signals.test_idioms.test_function_count += 1;
            }
            _ => {}
        }
    }
}

fn extract_identifier_arg(call_text: &str, head: &str) -> Option<String> {
    let trimmed = call_text.trim_start();
    let remainder = trimmed.strip_prefix(head)?.trim_start();
    let first = remainder
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(',')
        .trim_matches('(')
        .trim_matches(')')
        .trim();
    if first.is_empty() {
        None
    } else {
        Some(first.to_string())
    }
}

/// Build the Elixir language profile.
pub fn profile() -> LanguageProfile {
    let mut map = LanguageNodeMap::new();
    map.call_dispatch
        .insert("defmodule", vec![SignalAction::CallSemantics]);
    map.call_dispatch
        .insert("def", vec![SignalAction::CallSemantics]);
    map.call_dispatch
        .insert("defp", vec![SignalAction::CallSemantics]);
    map.call_dispatch
        .insert("import", vec![SignalAction::CallSemantics]);
    map.call_dispatch
        .insert("alias", vec![SignalAction::CallSemantics]);
    map.call_dispatch
        .insert("use", vec![SignalAction::CallSemantics]);
    map.call_dispatch
        .insert("require", vec![SignalAction::CallSemantics]);
    map.call_dispatch
        .insert("test", vec![SignalAction::CallSemantics]);

    LanguageProfile {
        node_map: map,
        semantics: Box::new(ElixirSemantics),
    }
}
