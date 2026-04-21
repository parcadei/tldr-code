use std::path::Path;

use tree_sitter::Node;

use super::super::language_profile::{
    create_evidence_from, node_text, LanguageNodeMap, LanguageProfile, LanguageSemantics,
    SignalAction,
};
use super::super::signals::{detect_naming_case, PatternSignals};

/// Semantic extractor for Ruby.
pub struct RubySemantics;

impl LanguageSemantics for RubySemantics {
    fn process_node(
        &self,
        node: Node,
        node_type: &str,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        match node_type {
            "class" => self.detect_class(node, source, file_path, signals),
            "method" | "singleton_method" => self.detect_method(node, source, file_path, signals),
            "begin" => self.detect_error_block(node, source, file_path, signals),
            _ => {}
        }
    }

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
            "require" => {
                if let Some(module) = extract_string_arg(&call_text) {
                    signals
                        .import_patterns
                        .absolute_imports
                        .push((module, file_path.display().to_string()));
                }
            }
            "require_relative" => {
                if let Some(module) = extract_string_arg(&call_text) {
                    signals
                        .import_patterns
                        .relative_imports
                        .push((module, file_path.display().to_string()));
                }
            }
            _ => {}
        }
    }
}

impl RubySemantics {
    fn detect_class(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, source);
            let case = detect_naming_case(&name);
            signals
                .naming
                .class_names
                .push((name, case, file_path.display().to_string()));
        }
    }

    fn detect_method(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, source);
            let case = detect_naming_case(&name);
            signals.naming.function_names.push((
                name.clone(),
                case,
                file_path.display().to_string(),
            ));
            if name.starts_with("test") {
                signals.test_idioms.test_function_count += 1;
            }
        }
    }

    fn detect_error_block(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        let text = node_text(node, source);
        if text.contains("rescue") {
            let evidence = create_evidence_from(node, source, file_path);
            signals.error_handling.try_catch_blocks.push(evidence);
        }
    }
}

fn extract_string_arg(call_text: &str) -> Option<String> {
    if let Some(start) = call_text.find('\'') {
        let rest = &call_text[start + 1..];
        if let Some(end) = rest.find('\'') {
            return Some(rest[..end].to_string());
        }
    }
    if let Some(start) = call_text.find('"') {
        let rest = &call_text[start + 1..];
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].to_string());
        }
    }
    None
}

/// Build the Ruby language profile.
pub fn profile() -> LanguageProfile {
    let mut map = LanguageNodeMap::new();
    map.dispatch
        .insert("class", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("method", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("singleton_method", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("begin", vec![SignalAction::CallSemantics]);

    map.call_dispatch
        .insert("require", vec![SignalAction::CallSemantics]);
    map.call_dispatch
        .insert("require_relative", vec![SignalAction::CallSemantics]);

    LanguageProfile {
        node_map: map,
        semantics: Box::new(RubySemantics),
    }
}
