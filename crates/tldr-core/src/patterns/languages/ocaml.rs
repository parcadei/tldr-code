use std::path::Path;

use tree_sitter::Node;

use super::super::language_profile::{
    node_text, LanguageNodeMap, LanguageProfile, LanguageSemantics, SignalAction, SignalTarget,
};
use super::super::signals::{detect_naming_case, PatternSignals};

/// Semantic extractor for OCaml.
pub struct OcamlSemantics;

impl LanguageSemantics for OcamlSemantics {
    fn process_node(
        &self,
        node: Node,
        node_type: &str,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        match node_type {
            "let_binding" | "value_definition" => {
                self.detect_let_binding(node, source, file_path, signals)
            }
            "module_definition" | "module_binding" => {
                self.detect_module(node, source, file_path, signals)
            }
            "open_statement" => self.detect_open(node, source, file_path, signals),
            "source_file" | "implementation" | "compilation_unit" => {
                self.detect_open_lines(source, file_path, signals)
            }
            _ => {}
        }
    }
}

impl OcamlSemantics {
    fn detect_let_binding(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        let mut cursor = node.walk();
        let has_params = node.child_by_field_name("parameter").is_some()
            || node
                .children(&mut cursor)
                .any(|c| c.kind() == "parameter" || c.kind() == "fun_expression");

        if has_params {
            if let Some(pattern_node) = node.child_by_field_name("pattern") {
                let raw_name = node_text(pattern_node, source);
                let name = raw_name
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    let case = detect_naming_case(&name);
                    signals.naming.function_names.push((
                        name.clone(),
                        case,
                        file_path.display().to_string(),
                    ));
                    if name.starts_with("test_") {
                        signals.test_idioms.test_function_count += 1;
                    }
                }
            }
        }
    }

    fn detect_module(
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
        } else {
            let text = node_text(node, source);
            if text.starts_with("module ") {
                let name = text
                    .trim_start_matches("module ")
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('=')
                    .to_string();
                if !name.is_empty() {
                    let case = detect_naming_case(&name);
                    signals
                        .naming
                        .class_names
                        .push((name, case, file_path.display().to_string()));
                }
            }
        }
    }

    fn detect_open(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        signals: &mut PatternSignals,
    ) {
        let text = node_text(node, source);
        let module = text.trim_start_matches("open ").trim().to_string();
        if !module.is_empty() {
            signals
                .import_patterns
                .absolute_imports
                .push((module, file_path.display().to_string()));
        }
    }

    fn detect_open_lines(&self, source: &str, file_path: &Path, signals: &mut PatternSignals) {
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("open ") {
                let module = trimmed.trim_start_matches("open ").trim().to_string();
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

/// Build the OCaml language profile.
pub fn profile() -> LanguageProfile {
    let mut map = LanguageNodeMap::new();
    map.dispatch
        .insert("let_binding", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("value_definition", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("module_definition", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("module_binding", vec![SignalAction::CallSemantics]);
    map.dispatch.insert(
        "try_expression",
        vec![SignalAction::PushEvidence(SignalTarget::TryCatchBlocks)],
    );
    map.dispatch
        .insert("open_statement", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("source_file", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("implementation", vec![SignalAction::CallSemantics]);
    map.dispatch
        .insert("compilation_unit", vec![SignalAction::CallSemantics]);

    LanguageProfile {
        node_map: map,
        semantics: Box::new(OcamlSemantics),
    }
}
