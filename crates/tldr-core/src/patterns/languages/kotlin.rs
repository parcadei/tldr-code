use std::path::Path;

use tree_sitter::Node;

use super::super::language_profile::{
    node_text, LanguageNodeMap, LanguageProfile, LanguageSemantics, SignalAction, SignalTarget,
};
use super::super::signals::{detect_naming_case, PatternSignals};

/// Semantic extractor for Kotlin.
pub struct KotlinSemantics;

impl LanguageSemantics for KotlinSemantics {
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
            "function_declaration" => self.detect_function(node, source, file_path, signals),
            "import_header" | "import_directive" => {
                self.detect_import(node, source, file_path, signals)
            }
            "source_file" => self.detect_import_lines(source, file_path, signals),
            _ => {}
        }
    }
}

impl KotlinSemantics {
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

    fn detect_import(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        let text = node_text(node, source);
        let module = text
            .trim_start_matches("import ")
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

    fn detect_import_lines(&self, source: &str, file_path: &Path, signals: &mut PatternSignals) {
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("import ") {
                let module = trimmed
                    .trim_start_matches("import ")
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
    }
}

/// Build the Kotlin language profile.
pub fn profile() -> LanguageProfile {
    let mut map = LanguageNodeMap::new();
    map.dispatch
        .insert("class_declaration", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("function_declaration", vec![SignalAction::CallSemantics]);
    map.dispatch.insert(
        "try_expression",
        vec![SignalAction::PushEvidence(SignalTarget::TryCatchBlocks)],
    );
    map.dispatch
        .insert("import_header", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("import_directive", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("source_file", vec![SignalAction::CallSemantics]);

    LanguageProfile {
        node_map: map,
        semantics: Box::new(KotlinSemantics),
    }
}
