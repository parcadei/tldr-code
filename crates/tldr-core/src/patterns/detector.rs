//! Pattern detector - Single-pass AST walker for pattern extraction
//!
//! This module provides the PatternDetector that walks an AST once and
//! collects signals for all pattern categories simultaneously.

use std::path::PathBuf;

use regex::Regex;
use tree_sitter::{Node, Tree};

use super::signals::*;
use crate::types::{Evidence, Language};

/// Pattern detector that extracts all pattern signals in a single AST pass
pub struct PatternDetector {
    language: Language,
    file_path: PathBuf,
}

impl PatternDetector {
    /// Create a new pattern detector for a specific language and file
    pub fn new(language: Language, file_path: PathBuf) -> Self {
        Self {
            language,
            file_path,
        }
    }

    /// Detect all patterns from a parsed AST tree
    pub fn detect_all(&self, tree: &Tree, source: &str) -> PatternSignals {
        let mut signals = PatternSignals::default();
        self.walk_node(tree.root_node(), source, &mut signals);
        signals
    }

    /// Fallback detection using regex for files with parse errors (A23 mitigation)
    pub fn detect_fallback(&self, source: &str) -> PatternSignals {
        let mut signals = PatternSignals::default();
        self.detect_fallback_patterns(source, &mut signals);
        signals
    }

    /// Recursively walk AST nodes and collect signals
    fn walk_node(&self, node: Node, source: &str, signals: &mut PatternSignals) {
        self.process_node(node, source, signals);

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_node(child, source, signals);
        }
    }

    /// Process a single AST node and extract signals
    fn process_node(&self, node: Node, source: &str, signals: &mut PatternSignals) {
        if let Some(profile) = super::language_profile::language_profile(self.language) {
            profile.process_node(node, source, &self.file_path, signals);
        }
    }

    /// Fallback pattern detection using regex (for files with parse errors)
    fn detect_fallback_patterns(&self, source: &str, signals: &mut PatternSignals) {
        // Soft delete patterns
        let is_deleted_re = Regex::new(r"(?i)(is_deleted|isDeleted)\s*[=:]").unwrap();
        let deleted_at_re = Regex::new(r"(?i)(deleted_at|deletedAt)\s*[=:]").unwrap();

        for (line_num, line) in source.lines().enumerate() {
            if is_deleted_re.is_match(line) {
                signals.soft_delete.is_deleted_fields.push(Evidence::new(
                    self.file_path.display().to_string(),
                    line_num as u32 + 1,
                    line.to_string(),
                ));
            }
            if deleted_at_re.is_match(line) {
                signals.soft_delete.deleted_at_fields.push(Evidence::new(
                    self.file_path.display().to_string(),
                    line_num as u32 + 1,
                    line.to_string(),
                ));
            }
        }

        // Error handling patterns
        if source.contains("try:") || source.contains("try {") {
            signals.error_handling.try_except_blocks.push(Evidence::new(
                self.file_path.display().to_string(),
                1,
                "try block detected".to_string(),
            ));
        }

        // Async patterns
        if source.contains("async ") || source.contains("await ") {
            signals.async_patterns.async_await.push(Evidence::new(
                self.file_path.display().to_string(),
                1,
                "async/await detected".to_string(),
            ));
        }
    }
}
