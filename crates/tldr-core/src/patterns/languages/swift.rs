use std::path::Path;

use tree_sitter::Node;

use super::super::language_profile::{
    create_evidence_from, node_text, LanguageNodeMap, LanguageProfile, LanguageSemantics,
    SignalAction, SignalTarget,
};
use super::super::signals::{detect_naming_case, PatternSignals};

/// Semantic extractor for Swift.
pub struct SwiftSemantics;

impl LanguageSemantics for SwiftSemantics {
    fn process_node(
        &self,
        node: Node,
        node_type: &str,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        match node_type {
            "class_declaration" | "protocol_declaration" => {
                self.detect_class_like(node, source, file_path, signals)
            }
            "function_declaration" => self.detect_function(node, source, file_path, signals),
            "import_declaration" => self.detect_import(node, source, file_path, signals),
            "do_statement" => self.detect_do_catch(node, source, file_path, signals),
            _ => {}
        }
    }
}

impl SwiftSemantics {
    fn detect_class_like(
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

    fn detect_import(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        let text = node_text(node, source);
        let module = text
            .trim_start_matches("import")
            .trim()
            .trim_end_matches(';')
            .to_string();
        if !module.is_empty() {
            signals
                .import_patterns
                .absolute_imports
                .push((module, file_path.display().to_string()));
        }
    }

    fn detect_do_catch(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        let text = node_text(node, source);
        if text.contains("catch") {
            let evidence = create_evidence_from(node, source, file_path);
            signals.error_handling.try_catch_blocks.push(evidence);
        }
    }
}

/// Build the Swift language profile.
pub fn profile() -> LanguageProfile {
    let mut map = LanguageNodeMap::new();
    map.dispatch
        .insert("class_declaration", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("function_declaration", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("protocol_declaration", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("import_declaration", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("do_statement", vec![SignalAction::CallSemantics]);
    map.dispatch.insert(
        "defer_statement",
        vec![SignalAction::PushEvidence(SignalTarget::DeferStatements)],
    );

    LanguageProfile {
        node_map: map,
        semantics: Box::new(SwiftSemantics),
    }
}
