use std::path::Path;

use tree_sitter::Node;

use super::super::language_profile::{
    node_text, LanguageNodeMap, LanguageProfile, LanguageSemantics, SignalAction, SignalTarget,
};
use super::super::signals::{detect_naming_case, PatternSignals};

/// Semantic extractor for C++.
pub struct CppSemantics;

impl LanguageSemantics for CppSemantics {
    fn process_node(
        &self,
        node: Node,
        node_type: &str,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        match node_type {
            "class_specifier" => self.detect_class(node, source, file_path, signals),
            "function_definition" => self.detect_function(node, source, file_path, signals),
            "preproc_include" => self.detect_include(node, source, file_path, signals),
            "namespace_definition" => self.detect_namespace(node, source, file_path, signals),
            _ => {}
        }
    }
}

impl CppSemantics {
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

    fn detect_namespace(
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

/// Build the C++ language profile.
pub fn profile() -> LanguageProfile {
    let mut map = LanguageNodeMap::new();
    map.dispatch
        .insert("class_specifier", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("function_definition", vec![SignalAction::CallSemantics]);
    map.dispatch.insert(
        "try_statement",
        vec![SignalAction::PushEvidence(SignalTarget::TryCatchBlocks)],
    );
    map.dispatch
        .insert("preproc_include", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("namespace_definition", vec![SignalAction::CallSemantics]);

    LanguageProfile {
        node_map: map,
        semantics: Box::new(CppSemantics),
    }
}
