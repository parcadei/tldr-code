//! Inheritance analysis module for class hierarchy extraction
//!
//! This module provides class hierarchy extraction and analysis for:
//! - Python classes (with ABC, Protocol, metaclass support - A12)
//! - TypeScript classes and interfaces
//! - Go struct embedding (modeled as composition - A14)
//! - Rust trait impl blocks (A16)
//! - Java classes, interfaces, enums, and records
//! - Kotlin classes, interfaces, objects, and data classes
//! - Scala classes, traits, objects, and case classes
//! - Swift classes, protocols, structs, and enums
//! - C# classes, interfaces, and structs
//! - Ruby classes and modules
//! - PHP classes, interfaces, and traits
//!
//! # Architecture
//!
//! 1. Extract classes from source files using tree-sitter
//! 2. Build inheritance graph with edges for extends/implements/embeds
//! 3. Detect patterns: ABC/Protocol, mixins, diamonds
//! 4. Resolve external bases (stdlib vs project vs unresolved)
//!
//! # Mitigations Addressed
//!
//! - A2: Diamond detection using BFS + set intersection (O(|ancestors|) not O(n^3))
//! - A12: Python metaclass extraction via keywords
//! - A14: Go struct embedding as Embeds edges
//! - A16: Rust trait impl blocks as Implements edges
//! - A17: --depth without --class validation
//! - A19: DOT output escaping for special characters
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::inheritance::{extract_inheritance, InheritanceOptions};
//!
//! let options = InheritanceOptions::default();
//! let report = extract_inheritance(Path::new("src"), Some(Language::Python), &options)?;
//! println!("Found {} classes", report.count);
//! ```

pub mod cpp; // real-repo-fixes-v1 (P9.BUG-R4): C/C++ inheritance extraction
pub mod csharp;
pub mod filter;
pub mod format;
pub mod go;
pub mod java;
pub mod kotlin;
pub mod patterns;
pub mod php;
pub mod python;
pub mod resolve;
pub mod ruby;
pub mod rust;
pub mod scala;
pub mod swift;
pub mod typescript;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::ast::parser::ParserPool;
use crate::walker::ProjectWalker;
use crate::error::TldrError;
use crate::types::{
    BaseResolution, InheritanceEdge, InheritanceGraph, InheritanceReport, Language,
};
use crate::TldrResult;

pub use filter::{filter_by_class, get_fuzzy_suggestions};
pub use format::{escape_dot_string, format_dot, format_text};
pub use patterns::{detect_abc_protocol, detect_diamonds, detect_mixins};
pub use resolve::{is_stdlib_class, resolve_base, PYTHON_STDLIB_CLASSES};

/// Options for inheritance analysis
#[derive(Debug, Clone, Default)]
pub struct InheritanceOptions {
    /// Filter to specific class (show ancestors + descendants)
    pub class_filter: Option<String>,
    /// Limit traversal depth (requires class_filter)
    pub depth: Option<usize>,
    /// Skip external base resolution
    pub no_external: bool,
    /// Skip ABC/mixin/diamond detection
    pub no_patterns: bool,
    /// Maximum nodes for DOT output (A39)
    pub max_nodes: Option<usize>,
    /// Cluster nodes by file in DOT output (A39)
    pub cluster_by_file: bool,
}

impl InheritanceOptions {
    /// Validate options - depth requires class_filter (A17)
    pub fn validate(&self) -> TldrResult<()> {
        if self.depth.is_some() && self.class_filter.is_none() {
            return Err(TldrError::InvalidArgs {
                arg: "--depth".to_string(),
                message: "--depth requires --class. Use --class <NAME> --depth N to limit traversal depth.".to_string(),
                suggestion: Some("To scan entire project without depth limit, omit --depth.".to_string()),
            });
        }
        Ok(())
    }
}

/// Main entry point for inheritance analysis
pub fn extract_inheritance(
    path: &Path,
    lang: Option<Language>,
    options: &InheritanceOptions,
) -> TldrResult<InheritanceReport> {
    // Validate options first (A17)
    options.validate()?;

    let start = Instant::now();
    let parser_pool = ParserPool::new();

    // Collect files matching language filter
    let files = collect_source_files(path, lang);
    if files.is_empty() {
        return Ok(InheritanceReport::new(path.to_path_buf()));
    }

    // Build inheritance graph
    let mut graph = InheritanceGraph::new();
    let mut languages_seen = HashSet::new();

    for file_path in &files {
        // real-repo-fixes-v1 (P9.BUG-R4): use sibling-aware detection so a
        // `.h` header next to `.cpp` translation units is parsed with the
        // C++ grammar. Without this, tinyxml2.h (8 obvious public-inherit
        // relations) was treated as plain C and contributed zero edges.
        let file_lang = Language::from_path_with_siblings(file_path)
            .or_else(|| Language::from_path(file_path))
            .unwrap_or(Language::Python);

        // Skip if language filter is specified and doesn't match
        if let Some(filter_lang) = lang {
            if file_lang != filter_lang {
                continue;
            }
        }

        languages_seen.insert(file_lang);

        // Extract classes based on language
        let source = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(_) => continue, // Skip unreadable files
        };

        let classes = match file_lang {
            Language::Python => python::extract_classes(&source, file_path, &parser_pool)?,
            Language::TypeScript | Language::JavaScript => {
                typescript::extract_classes(&source, file_path, &parser_pool)?
            }
            Language::Go => go::extract_classes(&source, file_path, &parser_pool)?,
            Language::Rust => rust::extract_classes(&source, file_path, &parser_pool)?,
            Language::Java => java::extract_classes(&source, file_path, &parser_pool)?,
            Language::Kotlin => kotlin::extract_classes(&source, file_path, &parser_pool)?,
            Language::Scala => scala::extract_classes(&source, file_path, &parser_pool)?,
            Language::Swift => swift::extract_classes(&source, file_path, &parser_pool)?,
            Language::CSharp => csharp::extract_classes(&source, file_path, &parser_pool)?,
            Language::Ruby => ruby::extract_classes(&source, file_path, &parser_pool)?,
            Language::Php => php::extract_classes(&source, file_path, &parser_pool)?,
            // real-repo-fixes-v1 (P9.BUG-R4): plug C/C++ inheritance.
            Language::Cpp => cpp::extract_classes(&source, file_path, &parser_pool)?,
            Language::C => cpp::extract_classes_c(&source, file_path, &parser_pool)?,
            _ => Vec::new(), // Unsupported language
        };

        // Add classes to graph
        for class in classes {
            let class_name = class.name.clone();
            let bases = class.bases.clone();

            graph.add_node(class);

            // Add edges for each base
            for base in bases {
                graph.add_edge(&class_name, &base);
            }
        }
    }

    // Resolve external bases unless disabled
    if !options.no_external {
        resolve::resolve_all_bases(&mut graph, path)?;
    }

    // Detect patterns unless disabled
    let diamonds = if options.no_patterns {
        Vec::new()
    } else {
        // Detect ABC/Protocol/Interface
        patterns::detect_abc_protocol(&mut graph);
        // Detect mixins
        patterns::detect_mixins(&mut graph);
        // Detect diamonds
        patterns::detect_diamonds(&graph)
    };

    // Apply class filter if specified
    let filtered_graph = if let Some(ref class_name) = options.class_filter {
        filter::filter_by_class(&graph, class_name, options.depth)?
    } else {
        graph
    };

    // Build report
    let mut report = InheritanceReport::new(path.to_path_buf());
    report.count = filtered_graph.nodes.len();
    report.languages = languages_seen.into_iter().collect();
    report.scan_time_ms = start.elapsed().as_millis() as u64;
    report.diamonds = diamonds;

    // Convert graph to edges and nodes for report
    report.nodes = filtered_graph.nodes.values().cloned().collect();
    report.edges = build_edges(&filtered_graph, path);
    report.roots = filtered_graph.find_roots();
    report.leaves = filtered_graph.find_leaves();

    Ok(report)
}

/// Collect source files matching the optional language filter
fn collect_source_files(path: &Path, lang: Option<Language>) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if path.is_file() {
        // Single file
        if let Some(file_lang) = Language::from_path(path) {
            if lang.is_none() || lang == Some(file_lang) {
                files.push(path.to_path_buf());
            }
        }
        return files;
    }

    // Walk directory with the shared ProjectWalker so `.gitignore` and the
    // default exclude list (vendor/, node_modules/, target/, ...) are honored.
    // A raw `walkdir::WalkDir` only skipped hidden entries and pulled in
    // `vendor/` on projects that commit their dependency tree, blowing up both
    // scan time and output size. Pass the language hint so the JS/TS-preserved
    // subset of the default excludes (build/, dist/, out/, bin/, obj/) is not
    // applied when the caller targets JavaScript/TypeScript.
    let walker = match lang {
        Some(language) => ProjectWalker::new(path).lang_hint(language),
        None => ProjectWalker::new(path),
    };
    for entry in walker.iter() {
        let entry_path = entry.path();

        // Skip non-files
        if !entry_path.is_file() {
            continue;
        }

        // Check language
        if let Some(file_lang) = Language::from_path(entry_path) {
            if lang.is_none() || lang == Some(file_lang) {
                files.push(entry_path.to_path_buf());
            }
        }
    }

    files
}

/// Build InheritanceEdge structs from graph
///
/// inheritance-and-dead-cleanup-v1 (M5): edges are deduplicated at the
/// (child, parent, parent_file) tuple level. The same heritage clause may
/// be emitted multiple times by language extractors (TS overload signatures,
/// TSX re-emission, Go interface satisfaction, etc.). Deduping here keeps
/// downstream consumers (and diamond detection counts) honest.
fn build_edges(graph: &InheritanceGraph, _project_root: &Path) -> Vec<InheritanceEdge> {
    let mut edges = Vec::new();
    let mut seen_edges: HashSet<(String, String, Option<PathBuf>)> = HashSet::new();

    for (child_name, parents) in &graph.parents {
        let child_node = match graph.nodes.get(child_name) {
            Some(n) => n,
            None => continue,
        };

        // Dedupe parent names per child (M5 dedup) — a child can have the same
        // parent listed multiple times when extractors emit the heritage
        // clause repeatedly. We preserve order via a HashSet-tracking pass.
        let mut seen_parents: HashSet<String> = HashSet::new();
        let parents: Vec<&String> = parents
            .iter()
            .filter(|p| seen_parents.insert((*p).clone()))
            .collect();

        for parent_name in parents {
            let parent_node = graph.nodes.get(parent_name);
            let (resolution, external) = if parent_node.is_some() {
                (BaseResolution::Project, false)
            } else if resolve::is_stdlib_class(parent_name, child_node.language) {
                (BaseResolution::Stdlib, true)
            } else {
                (BaseResolution::Unresolved, true)
            };

            let edge = if external {
                if resolution == BaseResolution::Stdlib {
                    InheritanceEdge::stdlib(
                        child_name,
                        parent_name,
                        child_node.file.clone(),
                        child_node.line,
                    )
                } else {
                    InheritanceEdge::unresolved(
                        child_name,
                        parent_name,
                        child_node.file.clone(),
                        child_node.line,
                    )
                }
            } else {
                let pn = parent_node.unwrap();
                InheritanceEdge::project(
                    child_name,
                    parent_name,
                    child_node.file.clone(),
                    child_node.line,
                    pn.file.clone(),
                    pn.line,
                )
            };

            // M5 dedup: (child, parent, parent_file) is the canonical edge
            // identity. Skip if we've already emitted this triple.
            let key = (
                edge.child.clone(),
                edge.parent.clone(),
                edge.parent_file.clone(),
            );
            if seen_edges.insert(key) {
                edges.push(edge);
            }
        }
    }

    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_options_validation_depth_without_class() {
        let options = InheritanceOptions {
            depth: Some(3),
            class_filter: None,
            ..Default::default()
        };

        let result = options.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("--depth requires --class"));
    }

    #[test]
    fn test_options_validation_depth_with_class() {
        let options = InheritanceOptions {
            depth: Some(3),
            class_filter: Some("MyClass".to_string()),
            ..Default::default()
        };

        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_extract_empty_project() {
        let dir = TempDir::new().unwrap();
        create_test_file(&dir, "empty.py", "# No classes here\npass\n");

        let options = InheritanceOptions::default();
        let report = extract_inheritance(dir.path(), Some(Language::Python), &options).unwrap();

        assert!(report.nodes.is_empty());
        assert!(report.edges.is_empty());
        assert_eq!(report.count, 0);
    }
}
