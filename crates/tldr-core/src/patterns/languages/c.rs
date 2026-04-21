use std::path::Path;

use tree_sitter::Node;

use super::super::language_profile::{
    node_text, LanguageNodeMap, LanguageProfile, LanguageSemantics, SignalAction,
};
use super::super::signals::{detect_naming_case, PatternSignals};

/// Semantic extractor for C.
pub struct CSemantics;

impl LanguageSemantics for CSemantics {
    fn process_node(
        &self,
        node: Node,
        node_type: &str,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        match node_type {
            "function_definition" => self.detect_function(node, source, file_path, signals),
            "preproc_include" => self.detect_include(node, source, file_path, signals),
            "preproc_def" => self.detect_define(node, source, file_path, signals),
            _ => {}
        }
    }
}

impl CSemantics {
    fn detect_function(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        let name = if let Some(decl) = node.child_by_field_name("declarator") {
            extract_name_from_declarator(&node_text(decl, source))
        } else {
            extract_name_from_declarator(&node_text(node, source))
        };

        if let Some(name) = name {
            let case = detect_naming_case(&name);
            signals
                .naming
                .function_names
                .push((name, case, file_path.display().to_string()));
        }
    }

    fn detect_include(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        let text = node_text(node, source);
        if let Some(module) = extract_include_module(&text) {
            signals
                .import_patterns
                .absolute_imports
                .push((module, file_path.display().to_string()));
        }
    }

    fn detect_define(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        let text = node_text(node, source);
        let tokens: Vec<&str> = text.split_whitespace().collect();
        if tokens.len() >= 2 {
            let name = tokens[1]
                .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_')
                .to_string();
            let case = detect_naming_case(&name);
            signals
                .naming
                .constant_names
                .push((name, case, file_path.display().to_string()));
        }
    }
}

fn extract_name_from_declarator(text: &str) -> Option<String> {
    let before_paren = text.split('(').next()?.trim();
    let name = before_paren
        .split_whitespace()
        .last()
        .unwrap_or("")
        .trim_matches('*')
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn extract_include_module(text: &str) -> Option<String> {
    if let Some(start) = text.find('<') {
        if let Some(end) = text[start + 1..].find('>') {
            return Some(text[start + 1..start + 1 + end].to_string());
        }
    }
    if let Some(start) = text.find('"') {
        let rest = &text[start + 1..];
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].to_string());
        }
    }
    None
}

/// Build the C language profile.
pub fn profile() -> LanguageProfile {
    let mut map = LanguageNodeMap::new();
    map.dispatch
        .insert("function_definition", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("preproc_include", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("preproc_def", vec![SignalAction::CallSemantics]);

    LanguageProfile {
        node_map: map,
        semantics: Box::new(CSemantics),
    }
}
