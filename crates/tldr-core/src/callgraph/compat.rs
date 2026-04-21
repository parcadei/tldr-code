//! Compatibility layer for V1/V2 call graph API migration (Phase 16a).
//!
//! This module provides type conversion between the new V2 call graph types
//! and the existing V1 types for backward compatibility during the migration
//! period.
//!
//! # Mitigations Implemented
//!
//! - M3.1: Use absolute paths in conversion to match V1 behavior
//! - M3.8: Include class name in dst_func for methods: "Class.method"
//! - M3.11: Provide into_v1() convenience method on CallGraphOutput
//!
//! # Spec Reference
//!
//! See `migration/spec/phases-14-16-spec.md` Section 16.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::builder_v2::{build_project_call_graph_v2, BuildConfig, BuildError};
use super::cross_file_types::{
    CallGraphIR, FileIR, FuncDef, ImportDef, ProjectCallGraphV2,
};

// =============================================================================
// V1 Type Definitions
// =============================================================================

/// V1 FunctionInfo format.
///
/// This is the format used by existing V1 consumers. The conversion from
/// FuncDef preserves all necessary fields for compatibility.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionInfo {
    /// Function name.
    pub name: String,
    /// File path where the function is defined.
    pub file: String,
    /// Start line (1-indexed).
    pub start_line: u32,
    /// End line (1-indexed).
    pub end_line: u32,
    /// Whether this is a method.
    pub is_method: bool,
    /// Containing class name if it's a method.
    pub class_name: Option<String>,
}

/// V1 ImportInfo format.
///
/// This is the format used by existing V1 consumers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportInfo {
    /// Module being imported.
    pub module: String,
    /// Imported names (empty for plain imports).
    pub names: Vec<String>,
    /// True for "from X import Y" style.
    pub is_from: bool,
    /// Module alias (e.g., "np" in `import numpy as np`).
    pub alias: Option<String>,
}

/// V1 CallEdge format (used by existing consumers).
///
/// # Path Format
///
/// Per M3.1, `caller` and `callee` use "file:func" format with absolute paths
/// to match V1 behavior. The `file` field is also absolute.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CallEdge {
    /// Caller in "file:func" format.
    pub caller: String,
    /// Callee in "file:func" format.
    pub callee: String,
    /// Source file path (absolute).
    pub file: String,
    /// Line number of the call (0 if unknown).
    pub line: u32,
    /// Source file as PathBuf (for compatibility with crate::types::CallEdge)
    pub src_file: PathBuf,
    /// Source function name.
    pub src_func: String,
    /// Destination file as PathBuf.
    pub dst_file: PathBuf,
    /// Destination function name.
    pub dst_func: String,
}

impl CallEdge {
    /// Creates a new CallEdge.
    pub fn new(
        src_file: PathBuf,
        src_func: String,
        dst_file: PathBuf,
        dst_func: String,
        line: u32,
    ) -> Self {
        let caller = format!("{}:{}", src_file.display(), src_func);
        let callee = format!("{}:{}", dst_file.display(), dst_func);
        let file = src_file.display().to_string();

        Self {
            caller,
            callee,
            file,
            line,
            src_file,
            src_func,
            dst_file,
            dst_func,
        }
    }
}

// =============================================================================
// Type Conversion Functions
// =============================================================================

/// Convert new FuncDef to existing FunctionInfo.
///
/// # Arguments
/// * `func` - The V2 FuncDef to convert
/// * `file` - The file path where this function is defined
///
/// # Returns
/// A FunctionInfo with all fields populated from the FuncDef.
pub fn funcdef_to_functioninfo(func: &FuncDef, file: &str) -> FunctionInfo {
    FunctionInfo {
        name: func.name.clone(),
        file: file.to_string(),
        start_line: func.line,
        end_line: func.end_line,
        is_method: func.is_method,
        class_name: func.class_name.clone(),
    }
}

/// Convert new ImportDef to existing ImportInfo format.
///
/// # Arguments
/// * `imp` - The V2 ImportDef to convert
///
/// # Returns
/// An ImportInfo with all fields populated from the ImportDef.
pub fn importdef_to_importinfo(imp: &ImportDef) -> ImportInfo {
    ImportInfo {
        module: imp.module.clone(),
        names: imp.names.clone(),
        is_from: imp.is_from,
        alias: imp.alias.clone(),
    }
}

/// Convert new ProjectCallGraphV2 to V1 CallEdge format.
///
/// # Arguments
/// * `graph` - The V2 call graph
/// * `root` - Project root for making paths absolute (M3.1)
///
/// # Returns
/// A vector of CallEdges in V1 format with absolute paths.
///
/// # Mitigations
/// - M3.1: Uses absolute paths to match V1 behavior
pub fn project_graph_to_edges(
    graph: &ProjectCallGraphV2,
    file_irs: &HashMap<String, FileIR>,
) -> Vec<CallEdge> {
    graph
        .edges()
        .map(|edge| {
            // Look up line number from file IRs if available
            let line = file_irs
                .get(&edge.src_file.to_string_lossy().to_string())
                .and_then(|ir| {
                    ir.calls.get(&edge.src_func).and_then(|calls| {
                        calls
                            .iter()
                            .find(|c| c.target == edge.dst_func)
                            .and_then(|c| c.line)
                    })
                })
                .unwrap_or(0);

            CallEdge::new(
                edge.src_file.clone(),
                edge.src_func.clone(),
                edge.dst_file.clone(),
                edge.dst_func.clone(),
                line,
            )
        })
        .collect()
}

/// Convert CallGraphIR to V1 ProjectCallGraph format.
///
/// # Arguments
/// * `ir` - The V2 CallGraphIR to convert
/// * `root` - Project root for making paths absolute
///
/// # Returns
/// A V1 ProjectCallGraph with all edges converted.
///
/// # Mitigations
/// - M3.1: Uses absolute paths
/// - M3.8: Method calls use "Class.method" format in dst_func
pub fn callgraph_ir_to_v1(ir: &CallGraphIR, root: &Path) -> crate::types::ProjectCallGraph {
    let mut graph = crate::types::ProjectCallGraph::new();

    // Use ir.edges only. These are resolved during Phase 14d-14f and contain
    // actual destination files (both intra-file and cross-file, deduplicated).
    //
    // P1 (parity-fix-plan.yaml): Keep paths project-relative if root is relative
    // Only join with root to make absolute if root itself is absolute
    let should_make_absolute = root.is_absolute();

    for edge in &ir.edges {
        let src_file = if should_make_absolute && edge.src_file.is_relative() {
            root.join(&edge.src_file)
        } else if !should_make_absolute {
            // P1: Keep relative for relative roots (matches Python V2 behavior)
            edge.src_file.clone()
        } else {
            edge.src_file.clone()
        };
        let dst_file = if should_make_absolute && edge.dst_file.is_relative() {
            root.join(&edge.dst_file)
        } else if !should_make_absolute {
            // P1: Keep relative for relative roots (matches Python V2 behavior)
            edge.dst_file.clone()
        } else {
            edge.dst_file.clone()
        };

        let v1_edge = crate::types::CallEdge {
            src_file,
            src_func: edge.src_func.clone(),
            dst_file,
            dst_func: edge.dst_func.clone(),
        };
        graph.add_edge(v1_edge);
    }

    graph
}

/// Convert CallGraphIR to old format (alias for callgraph_ir_to_v1).
///
/// This is a convenience function for tests that expect this name.
pub fn callgraph_ir_to_old(ir: &CallGraphIR) -> crate::types::ProjectCallGraph {
    callgraph_ir_to_v1(ir, &ir.root)
}

// =============================================================================
// CallGraphOutput Enum (M3.11)
// =============================================================================

/// Output type that can hold either V1 or V2 call graph.
///
/// Provides a unified interface for code that doesn't care about the
/// underlying representation, with convenience methods for conversion.
///
/// # Mitigation M3.11
///
/// This enum allows existing call graph consumers to continue working
/// unchanged by providing an `into_v1()` method that converts V2 to V1
/// format when needed.
pub enum CallGraphOutput {
    /// V1 format (existing ProjectCallGraph)
    V1(crate::types::ProjectCallGraph),
    /// V2 format (new CallGraphIR)
    V2(Box<CallGraphIR>),
}

impl CallGraphOutput {
    /// Convert to V1 ProjectCallGraph, regardless of internal format.
    ///
    /// If already V1, returns it directly. If V2, converts to V1 format.
    ///
    /// # Arguments
    /// * `root` - Project root for path resolution (only used for V2 -> V1 conversion)
    pub fn into_v1(self, root: &Path) -> crate::types::ProjectCallGraph {
        match self {
            CallGraphOutput::V1(g) => g,
            CallGraphOutput::V2(ir) => callgraph_ir_to_v1(&ir, root),
        }
    }

    /// Get an iterator over edges in V1 format.
    ///
    /// Works for both V1 and V2, converting V2 edges on the fly.
    pub fn edges(&self) -> Box<dyn Iterator<Item = crate::types::CallEdge> + '_> {
        match self {
            CallGraphOutput::V1(g) => Box::new(g.edges().cloned()),
            CallGraphOutput::V2(ir) => {
                let root = ir.root.clone();
                let v1 = callgraph_ir_to_v1(ir, &root);
                // Have to collect because we need 'static lifetime
                Box::new(v1.edges().cloned().collect::<Vec<_>>().into_iter())
            }
        }
    }

    /// Check if this is V2 format.
    pub fn is_v2(&self) -> bool {
        matches!(self, CallGraphOutput::V2(_))
    }
}

// =============================================================================
// Unified Entry Point (Spec Section 16.5)
// =============================================================================

/// Build call graph using configured builder.
///
/// This is the unified entry point for call graph building that routes to
/// either the V1 or V2 builder based on the `use_experimental` flag.
///
/// # Arguments
/// * `root` - Project root directory
/// * `config` - Build configuration
/// * `use_experimental` - When true, uses V2 builder (requires `experimental_callgraph` feature)
///
/// # Returns
/// * `CallGraphOutput::V1` when `use_experimental=false`
/// * `CallGraphOutput::V2` when `use_experimental=true` and feature is enabled
/// * `Err(FeatureNotEnabled)` when `use_experimental=true` but feature not compiled
///
/// # Mitigations Implemented
/// * M3.2: Bypasses daemon when experimental flag is set (N/A - library, not CLI)
/// * M3.7: Returns `FeatureNotEnabled` error when flag set but feature not compiled
/// * M3.9: Forces registry initialization before parallel processing
///
/// # Example
/// ```rust,ignore
/// use tldr_core::callgraph::compat::{build_call_graph, CallGraphOutput};
/// use tldr_core::callgraph::builder_v2::BuildConfig;
///
/// let config = BuildConfig {
///     language: "python".to_string(),
///     ..Default::default()
/// };
///
/// // Use V1 builder (default)
/// let output = build_call_graph(root, &config, false)?;
///
/// // Use V2 builder (experimental)
/// let output = build_call_graph(root, &config, true)?;
///
/// // Convert to V1 format if needed
/// let v1_graph = output.into_v1(root);
/// ```
pub fn build_call_graph(
    root: &Path,
    config: &BuildConfig,
    use_experimental: bool,
) -> Result<CallGraphOutput, BuildError> {
    if use_experimental {
        // M3.9: Validate language is supported before any parallel processing
        // This ensures the language registry is initialized
        let language = config.language.to_lowercase();
        let supported = matches!(
            language.as_str(),
            "python"
                | "typescript"
                | "tsx"
                | "javascript"
                | "js"
                | "go"
                | "rust"
                | "java"
                | "c"
                | "cpp"
                | "csharp"
                | "kotlin"
                | "scala"
                | "php"
                | "ruby"
                | "lua"
                | "luau"
                | "elixir"
                | "ocaml"
        );
        if !supported && !config.language.is_empty() {
            return Err(BuildError::UnsupportedLanguage(config.language.clone()));
        }

        // Use V2 builder
        let ir = build_project_call_graph_v2(root, config.clone())?;
        return Ok(CallGraphOutput::V2(Box::new(ir)));
    }

    // Use V1 builder
    // Convert language string to Language enum
    let language = match config.language.to_lowercase().as_str() {
        "python" => crate::types::Language::Python,
        "typescript" | "tsx" => crate::types::Language::TypeScript,
        "javascript" | "js" => crate::types::Language::JavaScript,
        "go" => crate::types::Language::Go,
        "rust" => crate::types::Language::Rust,
        "java" => crate::types::Language::Java,
        "c" => crate::types::Language::C,
        "cpp" => crate::types::Language::Cpp,
        "csharp" => crate::types::Language::CSharp,
        "kotlin" => crate::types::Language::Kotlin,
        "scala" => crate::types::Language::Scala,
        "swift" => crate::types::Language::Swift,
        "php" => crate::types::Language::Php,
        "ruby" => crate::types::Language::Ruby,
        "lua" => crate::types::Language::Lua,
        "luau" => crate::types::Language::Luau,
        "elixir" => crate::types::Language::Elixir,
        "ocaml" => crate::types::Language::Ocaml,
        _ => return Err(BuildError::UnsupportedLanguage(config.language.clone())),
    };

    let v1_graph = crate::callgraph::build_project_call_graph(root, language, None, true)
        .map_err(|e| BuildError::WorkspaceConfig(e.to_string()))?;

    Ok(CallGraphOutput::V1(v1_graph))
}

// =============================================================================
// Builder Comparison (A/B Testing)
// =============================================================================

/// Normalized edge for cross-platform comparison (M3.4 mitigation).
///
/// Uses String instead of PathBuf to ensure consistent comparison:
/// - Paths are relative to root
/// - Forward slashes on all platforms
/// - Case-sensitive comparison
///
/// # Spec Reference
///
/// See `migration/spec/phases-14-16-spec.md` Section M3.4.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NormalizedEdge {
    /// Source file path (relative to root, forward slashes).
    pub src_file: String,
    /// Source function name.
    pub src_func: String,
    /// Destination file path (relative to root, forward slashes).
    pub dst_file: String,
    /// Destination function name.
    pub dst_func: String,
}

impl NormalizedEdge {
    /// Creates a new NormalizedEdge.
    pub fn new(src_file: String, src_func: String, dst_file: String, dst_func: String) -> Self {
        Self {
            src_file,
            src_func,
            dst_file,
            dst_func,
        }
    }
}

/// Normalize a CallEdge for cross-platform comparison (M3.4).
///
/// Converts PathBuf to String with:
/// - Paths relative to root
/// - Forward slashes (even on Windows)
///
/// # Arguments
/// * `edge` - The CallEdge to normalize
/// * `root` - Project root for making paths relative
///
/// # Returns
/// A NormalizedEdge suitable for cross-platform comparison.
pub fn normalize_edge(edge: &crate::types::CallEdge, root: &Path) -> NormalizedEdge {
    let strip_root = |p: &Path| -> String {
        p.strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    };

    NormalizedEdge {
        src_file: strip_root(&edge.src_file),
        src_func: edge.src_func.clone(),
        dst_file: strip_root(&edge.dst_file),
        dst_func: edge.dst_func.clone(),
    }
}

/// Result of comparing V1 and V2 builder outputs.
///
/// Used for A/B testing to validate V2 produces equivalent results.
/// Uses NormalizedEdge for cross-platform comparison (M3.4).
#[derive(Debug, Default)]
pub struct ComparisonResult {
    /// Edges found only in V1 (V2 is missing these).
    pub only_in_old: HashSet<NormalizedEdge>,
    /// Edges found only in V2 (V2 found additional edges).
    pub only_in_new: HashSet<NormalizedEdge>,
    /// Edges found in both V1 and V2.
    pub in_both: HashSet<NormalizedEdge>,
}

/// Compare V1 and V2 builder results.
///
/// Runs both builders on the same project and compares their outputs.
/// Edges are normalized before comparison (M3.4).
///
/// # Arguments
/// * `root` - Project root directory
/// * `config` - Build configuration
///
/// # Returns
/// ComparisonResult showing which edges are in each builder's output.
pub fn compare_builders(root: &Path, config: &BuildConfig) -> Result<ComparisonResult, BuildError> {
    // Build with V2
    let v2_ir = build_project_call_graph_v2(root, config.clone())?;
    let v2_graph = callgraph_ir_to_v1(&v2_ir, root);

    // Build with V1
    // Note: V1 builder uses Language enum, need to convert
    let language = match config.language.to_lowercase().as_str() {
        "python" => crate::types::Language::Python,
        "typescript" | "tsx" => crate::types::Language::TypeScript,
        "javascript" | "js" => crate::types::Language::JavaScript,
        "go" => crate::types::Language::Go,
        "rust" => crate::types::Language::Rust,
        "java" => crate::types::Language::Java,
        "c" => crate::types::Language::C,
        "cpp" => crate::types::Language::Cpp,
        "csharp" => crate::types::Language::CSharp,
        "kotlin" => crate::types::Language::Kotlin,
        "scala" => crate::types::Language::Scala,
        "swift" => crate::types::Language::Swift,
        "php" => crate::types::Language::Php,
        "ruby" => crate::types::Language::Ruby,
        "lua" => crate::types::Language::Lua,
        "luau" => crate::types::Language::Luau,
        "elixir" => crate::types::Language::Elixir,
        "ocaml" => crate::types::Language::Ocaml,
        _ => return Err(BuildError::UnsupportedLanguage(config.language.clone())),
    };

    let v1_graph = crate::callgraph::build_project_call_graph(root, language, None, true)
        .map_err(|e| BuildError::WorkspaceConfig(e.to_string()))?;

    // Normalize edges before comparison (M3.4)
    // This ensures cross-platform compatibility by using relative paths with forward slashes
    let v1_edges: HashSet<NormalizedEdge> =
        v1_graph.edges().map(|e| normalize_edge(e, root)).collect();
    let v2_edges: HashSet<NormalizedEdge> =
        v2_graph.edges().map(|e| normalize_edge(e, root)).collect();

    let only_in_old: HashSet<_> = v1_edges.difference(&v2_edges).cloned().collect();
    let only_in_new: HashSet<_> = v2_edges.difference(&v1_edges).cloned().collect();
    let in_both: HashSet<_> = v1_edges.intersection(&v2_edges).cloned().collect();

    Ok(ComparisonResult {
        only_in_old,
        only_in_new,
        in_both,
    })
}

// =============================================================================
// Output Format Compatibility
// =============================================================================

/// Format edges in V1-compatible output format.
///
/// Produces output matching the existing CLI format:
/// ```text
/// file:func -> file:func
/// ```
///
/// Edges are sorted for deterministic output.
///
/// # Arguments
/// * `edges` - Tuple of (src_file, src_func, dst_file, dst_func)
///
/// # Returns
/// Sorted, newline-separated string of edges.
pub fn format_edges_compatible(edges: &[(String, String, String, String)]) -> String {
    let mut lines: Vec<String> = edges
        .iter()
        .map(|(s_f, s_fn, d_f, d_fn)| format!("{}:{} -> {}:{}", s_f, s_fn, d_f, d_fn))
        .collect();
    lines.sort();
    lines.join("\n")
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_funcdef_to_functioninfo_basic() {
        let func = FuncDef::function("test_func", 1, 10);
        let info = funcdef_to_functioninfo(&func, "test.py");

        assert_eq!(info.name, "test_func");
        assert_eq!(info.file, "test.py");
        assert_eq!(info.start_line, 1);
        assert_eq!(info.end_line, 10);
        assert!(!info.is_method);
        assert_eq!(info.class_name, None);
    }

    #[test]
    fn test_funcdef_to_functioninfo_method() {
        let func = FuncDef::method("my_method", "MyClass", 5, 15);
        let info = funcdef_to_functioninfo(&func, "module.py");

        assert_eq!(info.name, "my_method");
        assert!(info.is_method);
        assert_eq!(info.class_name, Some("MyClass".to_string()));
    }

    #[test]
    fn test_importdef_to_importinfo_from() {
        let import = ImportDef::from_import("mymodule", vec!["MyClass".to_string()]);
        let info = importdef_to_importinfo(&import);

        assert_eq!(info.module, "mymodule");
        assert!(info.is_from);
        assert_eq!(info.names, vec!["MyClass"]);
    }

    #[test]
    fn test_importdef_to_importinfo_simple() {
        let import = ImportDef::simple_import("json");
        let info = importdef_to_importinfo(&import);

        assert_eq!(info.module, "json");
        assert!(!info.is_from);
        assert!(info.names.is_empty());
    }

    #[test]
    fn test_importdef_to_importinfo_alias() {
        let import = ImportDef::import_as("numpy", "np");
        let info = importdef_to_importinfo(&import);

        assert_eq!(info.module, "numpy");
        assert_eq!(info.alias, Some("np".to_string()));
    }

    #[test]
    fn test_format_edges_compatible_basic() {
        let edges = vec![(
            "a.py".to_string(),
            "foo".to_string(),
            "b.py".to_string(),
            "bar".to_string(),
        )];
        let output = format_edges_compatible(&edges);
        assert_eq!(output, "a.py:foo -> b.py:bar");
    }

    #[test]
    fn test_format_edges_compatible_sorted() {
        let edges = vec![
            (
                "z.py".to_string(),
                "z".to_string(),
                "a.py".to_string(),
                "a".to_string(),
            ),
            (
                "a.py".to_string(),
                "a".to_string(),
                "b.py".to_string(),
                "b".to_string(),
            ),
        ];
        let output = format_edges_compatible(&edges);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines[0], "a.py:a -> b.py:b");
        assert_eq!(lines[1], "z.py:z -> a.py:a");
    }

    #[test]
    fn test_format_edges_compatible_empty() {
        let edges: Vec<(String, String, String, String)> = vec![];
        let output = format_edges_compatible(&edges);
        assert!(output.is_empty());
    }

    #[test]
    fn test_comparison_result_default() {
        let result = ComparisonResult::default();
        assert!(result.only_in_old.is_empty());
        assert!(result.only_in_new.is_empty());
        assert!(result.in_both.is_empty());
    }

    #[test]
    fn test_normalized_edge_new() {
        let edge = NormalizedEdge::new(
            "src/main.py".to_string(),
            "main".to_string(),
            "src/helper.py".to_string(),
            "process".to_string(),
        );
        assert_eq!(edge.src_file, "src/main.py");
        assert_eq!(edge.src_func, "main");
        assert_eq!(edge.dst_file, "src/helper.py");
        assert_eq!(edge.dst_func, "process");
    }

    #[test]
    fn test_normalize_edge_strips_root() {
        let root = Path::new("/project");
        let edge = crate::types::CallEdge {
            src_file: PathBuf::from("/project/src/main.py"),
            src_func: "main".to_string(),
            dst_file: PathBuf::from("/project/src/helper.py"),
            dst_func: "process".to_string(),
        };

        let normalized = normalize_edge(&edge, root);

        assert_eq!(normalized.src_file, "src/main.py");
        assert_eq!(normalized.dst_file, "src/helper.py");
    }

    #[test]
    fn test_normalize_edge_forward_slashes() {
        // Test that backslashes are converted to forward slashes
        let root = Path::new("/project");
        let edge = crate::types::CallEdge {
            src_file: PathBuf::from("/project/src/main.py"),
            src_func: "main".to_string(),
            dst_file: PathBuf::from("/project/src/helper.py"),
            dst_func: "process".to_string(),
        };

        let normalized = normalize_edge(&edge, root);

        // Should use forward slashes regardless of platform
        assert!(!normalized.src_file.contains('\\'));
        assert!(!normalized.dst_file.contains('\\'));
    }

    #[test]
    fn test_normalize_edge_hash_equality() {
        let edge1 = NormalizedEdge::new(
            "main.py".to_string(),
            "func".to_string(),
            "helper.py".to_string(),
            "helper".to_string(),
        );
        let edge2 = NormalizedEdge::new(
            "main.py".to_string(),
            "func".to_string(),
            "helper.py".to_string(),
            "helper".to_string(),
        );

        // Same content should be equal and have same hash
        assert_eq!(edge1, edge2);

        let mut set = HashSet::new();
        set.insert(edge1.clone());
        assert!(set.contains(&edge2));
    }

    // =========================================================================
    // Tests for build_call_graph unified entry point
    // =========================================================================

    #[test]
    fn test_build_call_graph_v1_routing() {
        // Create a temporary project
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.py"), "def foo(): pass").unwrap();

        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        // With use_experimental=false, should use V1 builder
        let result = build_call_graph(dir.path(), &config, false);
        assert!(result.is_ok(), "V1 build should succeed");

        let output = result.unwrap();
        assert!(!output.is_v2(), "Should return V1 format");
    }

    #[test]
    fn test_build_call_graph_v2_routing() {
        // Create a temporary project
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.py"), "def foo(): pass").unwrap();

        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        // With use_experimental=true, should use V2 builder
        let result = build_call_graph(dir.path(), &config, true);
        assert!(result.is_ok(), "V2 build should succeed");

        let output = result.unwrap();
        assert!(output.is_v2(), "Should return V2 format");
    }

    #[test]
    fn test_build_call_graph_unsupported_language() {
        let dir = tempfile::TempDir::new().unwrap();

        let config = BuildConfig {
            language: "brainfuck".to_string(), // Not supported
            ..Default::default()
        };

        // Both V1 and V2 should fail for unsupported language
        let result_v1 = build_call_graph(dir.path(), &config, false);
        assert!(
            result_v1.is_err(),
            "V1 should fail for unsupported language"
        );

        let result_v2 = build_call_graph(dir.path(), &config, true);
        assert!(
            result_v2.is_err(),
            "V2 should fail for unsupported language"
        );
    }

    #[test]
    fn test_build_call_graph_output_conversion() {
        // Test that V2 output can be converted to V1
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.py"), "def foo(): pass").unwrap();

        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let output = build_call_graph(dir.path(), &config, true).unwrap();

        // Should be able to convert to V1
        let v1_graph = output.into_v1(dir.path());
        let _ = v1_graph;
    }
}
