use std::path::Path;

use tree_sitter::Node;

use super::super::language_profile::{
    node_text, LanguageNodeMap, LanguageProfile, LanguageSemantics, SignalAction, SignalTarget,
};
use super::super::signals::{detect_naming_case, PatternSignals};

/// Semantic extractor for C#.
pub struct CSharpSemantics;

impl LanguageSemantics for CSharpSemantics {
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
            "method_declaration" => self.detect_method(node, source, file_path, signals),
            "using_directive" => self.detect_using(node, source, file_path, signals),
            _ => {}
        }
    }
}

impl CSharpSemantics {
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
            signals
                .naming
                .function_names
                .push((name, case, file_path.display().to_string()));
        }
    }

    fn detect_using(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        let text = node_text(node, source);
        let module = text
            .trim_start_matches("using")
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
}

/// Build the C# language profile.
pub fn profile() -> LanguageProfile {
    let mut map = LanguageNodeMap::new();
    map.dispatch
        .insert("class_declaration", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("method_declaration", vec![SignalAction::CallSemantics]);
    map.dispatch.insert(
        "try_statement",
        vec![SignalAction::PushEvidence(SignalTarget::TryCatchBlocks)],
    );
    map.dispatch
        .insert("using_directive", vec![SignalAction::CallSemantics]);
    map.dispatch.insert(
        "await_expression",
        vec![SignalAction::PushEvidence(SignalTarget::AsyncAwait)],
    );

    LanguageProfile {
        node_map: map,
        semantics: Box::new(CSharpSemantics),
    }
}
