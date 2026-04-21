use std::path::Path;

use tree_sitter::Node;

use super::super::language_profile::{
    node_text, LanguageNodeMap, LanguageProfile, LanguageSemantics, SignalAction, SignalTarget,
};
use super::super::signals::{detect_naming_case, PatternSignals};

/// Semantic extractor for Scala.
pub struct ScalaSemantics;

impl LanguageSemantics for ScalaSemantics {
    fn process_node(
        &self,
        node: Node,
        node_type: &str,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        match node_type {
            "class_definition" | "object_definition" | "trait_definition" => {
                self.detect_class_like(node, source, file_path, signals)
            }
            "function_definition" => self.detect_function(node, source, file_path, signals),
            "val_definition" => self.detect_val(node, source, file_path, signals),
            "import_declaration" => self.detect_import(node, source, file_path, signals),
            _ => {}
        }
    }
}

impl ScalaSemantics {
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

    fn detect_val(&self, node: Node, source: &str, file_path: &Path, signals: &mut PatternSignals) {
        let text = node_text(node, source);
        if text.starts_with("val ") {
            let name = text
                .trim_start_matches("val ")
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches(':')
                .to_string();
            if !name.is_empty() {
                let case = detect_naming_case(&name);
                signals
                    .naming
                    .constant_names
                    .push((name, case, file_path.display().to_string()));
            }
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
}

/// Build the Scala language profile.
pub fn profile() -> LanguageProfile {
    let mut map = LanguageNodeMap::new();
    map.dispatch
        .insert("class_definition", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("object_definition", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("trait_definition", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("function_definition", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("val_definition", vec![SignalAction::CallSemantics]);
    map.dispatch.insert(
        "try_expression",
        vec![SignalAction::PushEvidence(SignalTarget::TryCatchBlocks)],
    );
    map.dispatch
        .insert("import_declaration", vec![SignalAction::CallSemantics]);

    LanguageProfile {
        node_map: map,
        semantics: Box::new(ScalaSemantics),
    }
}
