use std::path::Path;

use tree_sitter::Node;

use super::super::language_profile::{
    node_text, LanguageNodeMap, LanguageProfile, LanguageSemantics, SignalAction, SignalTarget,
};
use super::super::signals::{detect_naming_case, PatternSignals};

/// Semantic extractor for PHP.
pub struct PhpSemantics;

impl LanguageSemantics for PhpSemantics {
    fn process_node(
        &self,
        node: Node,
        node_type: &str,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        match node_type {
            "class_declaration" => self.detect_class(node, source, file_path, signals),
            "method_declaration" | "function_definition" => {
                self.detect_function(node, source, file_path, signals)
            }
            "use_declaration" | "namespace_use_declaration" | "namespace_use_clause" => {
                self.detect_use_import(node, source, file_path, signals)
            }
            _ => {}
        }
    }
}

impl PhpSemantics {
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
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, source);
            let case = detect_naming_case(&name);
            signals
                .naming
                .function_names
                .push((name, case, file_path.display().to_string()));
        }
    }

    fn detect_use_import(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        let text = node_text(node, source);
        if let Some(idx) = text.find("use ") {
            let module = text[idx + 4..].trim().trim_end_matches(';').to_string();
            if !module.is_empty() {
                signals
                    .import_patterns
                    .absolute_imports
                    .push((module, file_path.display().to_string()));
            }
        }
    }
}

/// Build the PHP language profile.
pub fn profile() -> LanguageProfile {
    let mut map = LanguageNodeMap::new();
    map.dispatch
        .insert("class_declaration", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("method_declaration", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("function_definition", vec![SignalAction::CallSemantics]);
    map.dispatch.insert(
        "try_statement",
        vec![SignalAction::PushEvidence(SignalTarget::TryCatchBlocks)],
    );
    map.dispatch
        .insert("use_declaration", vec![SignalAction::CallSemantics]);
    map.dispatch.insert(
        "namespace_use_declaration",
        vec![SignalAction::CallSemantics],
    );
    map.dispatch
        .insert("namespace_use_clause", vec![SignalAction::CallSemantics]);

    LanguageProfile {
        node_map: map,
        semantics: Box::new(PhpSemantics),
    }
}
