//! Test coverage for tldr-core types module
//!
//! Tests all public types, enums, and their methods from:
//! - crates/tldr-core/src/types.rs

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;

// Import the types from tldr_core
use tldr_core::types::*;

// =============================================================================
// Language Enum Tests
// =============================================================================

#[test]
fn test_language_all_variants() {
    let all = Language::all();
    assert_eq!(all.len(), 18);

    // Verify all expected languages are present
    let expected = vec![
        Language::Python,
        Language::TypeScript,
        Language::JavaScript,
        Language::Go,
        Language::Rust,
        Language::Java,
        Language::C,
        Language::Cpp,
        Language::Ruby,
        Language::Kotlin,
        Language::Swift,
        Language::CSharp,
        Language::Scala,
        Language::Php,
        Language::Lua,
        Language::Luau,
        Language::Elixir,
        Language::Ocaml,
    ];

    for lang in &expected {
        assert!(all.contains(lang), "Missing language: {:?}", lang);
    }
}

#[test]
fn test_language_extensions() {
    // P0 Languages
    assert_eq!(Language::Python.extensions(), &[".py"]);
    assert_eq!(Language::TypeScript.extensions(), &[".ts", ".tsx"]);
    assert_eq!(
        Language::JavaScript.extensions(),
        &[".js", ".jsx", ".mjs", ".cjs"]
    );
    assert_eq!(Language::Go.extensions(), &[".go"]);

    // P1 Languages
    assert_eq!(Language::Rust.extensions(), &[".rs"]);
    assert_eq!(Language::Java.extensions(), &[".java"]);

    // P2 Languages
    assert_eq!(Language::C.extensions(), &[".c", ".h"]);
    assert_eq!(Language::Cpp.extensions(), &[".cpp", ".cc", ".cxx", ".hpp"]);
    assert_eq!(Language::Ruby.extensions(), &[".rb"]);
    assert_eq!(Language::Kotlin.extensions(), &[".kt", ".kts"]);
    assert_eq!(Language::Swift.extensions(), &[".swift"]);
    assert_eq!(Language::CSharp.extensions(), &[".cs"]);
    assert_eq!(Language::Scala.extensions(), &[".scala"]);
    assert_eq!(Language::Php.extensions(), &[".php"]);
    assert_eq!(Language::Lua.extensions(), &[".lua"]);
    assert_eq!(Language::Luau.extensions(), &[".luau"]);
    assert_eq!(Language::Elixir.extensions(), &[".ex", ".exs"]);
    assert_eq!(Language::Ocaml.extensions(), &[".ml", ".mli"]);
}

#[test]
fn test_language_from_extension_with_dot() {
    assert_eq!(Language::from_extension(".py"), Some(Language::Python));
    assert_eq!(Language::from_extension(".ts"), Some(Language::TypeScript));
    assert_eq!(Language::from_extension(".tsx"), Some(Language::TypeScript));
    assert_eq!(Language::from_extension(".rs"), Some(Language::Rust));
}

#[test]
fn test_language_from_extension_without_dot() {
    assert_eq!(Language::from_extension("py"), Some(Language::Python));
    assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
    assert_eq!(Language::from_extension("go"), Some(Language::Go));
}

#[test]
fn test_language_from_extension_case_insensitive() {
    assert_eq!(Language::from_extension(".PY"), Some(Language::Python));
    assert_eq!(Language::from_extension(".Py"), Some(Language::Python));
    assert_eq!(Language::from_extension(".TS"), Some(Language::TypeScript));
}

#[test]
fn test_language_from_extension_unknown() {
    assert_eq!(Language::from_extension(".xyz"), None);
    assert_eq!(Language::from_extension(".unknown"), None);
    assert_eq!(Language::from_extension(""), None);
}

#[test]
fn test_language_from_path() {
    assert_eq!(
        Language::from_path(std::path::Path::new("/path/to/file.py")),
        Some(Language::Python)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("app.ts")),
        Some(Language::TypeScript)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("/deep/nested/path/lib.rs")),
        Some(Language::Rust)
    );
}

#[test]
fn test_language_from_path_no_extension() {
    assert_eq!(Language::from_path(std::path::Path::new("Makefile")), None);
    assert_eq!(
        Language::from_path(std::path::Path::new("/path/to/file")),
        None
    );
}

#[test]
fn test_language_from_path_directory() {
    assert_eq!(
        Language::from_path(std::path::Path::new("/path/to/dir/")),
        None
    );
}

#[test]
fn test_language_as_str() {
    assert_eq!(Language::Python.as_str(), "python");
    assert_eq!(Language::TypeScript.as_str(), "typescript");
    assert_eq!(Language::JavaScript.as_str(), "javascript");
    assert_eq!(Language::Go.as_str(), "go");
    assert_eq!(Language::Rust.as_str(), "rust");
    assert_eq!(Language::Java.as_str(), "java");
    assert_eq!(Language::C.as_str(), "c");
    assert_eq!(Language::Cpp.as_str(), "cpp");
    assert_eq!(Language::Ruby.as_str(), "ruby");
    assert_eq!(Language::Kotlin.as_str(), "kotlin");
    assert_eq!(Language::Swift.as_str(), "swift");
    assert_eq!(Language::CSharp.as_str(), "csharp");
    assert_eq!(Language::Scala.as_str(), "scala");
    assert_eq!(Language::Php.as_str(), "php");
    assert_eq!(Language::Lua.as_str(), "lua");
    assert_eq!(Language::Luau.as_str(), "luau");
    assert_eq!(Language::Elixir.as_str(), "elixir");
    assert_eq!(Language::Ocaml.as_str(), "ocaml");
}

#[test]
fn test_language_display() {
    assert_eq!(format!("{}", Language::Python), "python");
    assert_eq!(format!("{}", Language::Rust), "rust");
}

#[test]
fn test_language_from_str_valid() {
    assert_eq!(Language::from_str("python").unwrap(), Language::Python);
    assert_eq!(Language::from_str("py").unwrap(), Language::Python);
    assert_eq!(
        Language::from_str("typescript").unwrap(),
        Language::TypeScript
    );
    assert_eq!(Language::from_str("ts").unwrap(), Language::TypeScript);
    assert_eq!(
        Language::from_str("javascript").unwrap(),
        Language::JavaScript
    );
    assert_eq!(Language::from_str("js").unwrap(), Language::JavaScript);
    assert_eq!(Language::from_str("go").unwrap(), Language::Go);
    assert_eq!(Language::from_str("golang").unwrap(), Language::Go);
    assert_eq!(Language::from_str("rust").unwrap(), Language::Rust);
    assert_eq!(Language::from_str("rs").unwrap(), Language::Rust);
    assert_eq!(Language::from_str("java").unwrap(), Language::Java);
    assert_eq!(Language::from_str("c").unwrap(), Language::C);
    assert_eq!(Language::from_str("cpp").unwrap(), Language::Cpp);
    assert_eq!(Language::from_str("c++").unwrap(), Language::Cpp);
    assert_eq!(Language::from_str("cxx").unwrap(), Language::Cpp);
    assert_eq!(Language::from_str("ruby").unwrap(), Language::Ruby);
    assert_eq!(Language::from_str("rb").unwrap(), Language::Ruby);
    assert_eq!(Language::from_str("kotlin").unwrap(), Language::Kotlin);
    assert_eq!(Language::from_str("kt").unwrap(), Language::Kotlin);
    assert_eq!(Language::from_str("swift").unwrap(), Language::Swift);
    assert_eq!(Language::from_str("csharp").unwrap(), Language::CSharp);
    assert_eq!(Language::from_str("c#").unwrap(), Language::CSharp);
    assert_eq!(Language::from_str("cs").unwrap(), Language::CSharp);
    assert_eq!(Language::from_str("scala").unwrap(), Language::Scala);
    assert_eq!(Language::from_str("php").unwrap(), Language::Php);
    assert_eq!(Language::from_str("lua").unwrap(), Language::Lua);
    assert_eq!(Language::from_str("luau").unwrap(), Language::Luau);
    assert_eq!(Language::from_str("elixir").unwrap(), Language::Elixir);
    assert_eq!(Language::from_str("ex").unwrap(), Language::Elixir);
    assert_eq!(Language::from_str("ocaml").unwrap(), Language::Ocaml);
    assert_eq!(Language::from_str("ml").unwrap(), Language::Ocaml);
}

#[test]
fn test_language_from_str_invalid() {
    assert!(Language::from_str("unknown").is_err());
    assert!(Language::from_str("").is_err());
    assert!(Language::from_str("cppp").is_err());
}

#[test]
fn test_language_is_p0() {
    assert!(Language::Python.is_p0());
    assert!(Language::TypeScript.is_p0());
    assert!(Language::JavaScript.is_p0());
    assert!(Language::Go.is_p0());

    assert!(!Language::Rust.is_p0());
    assert!(!Language::Java.is_p0());
    assert!(!Language::C.is_p0());
}

#[test]
fn test_language_is_p1() {
    assert!(Language::Rust.is_p1());
    assert!(Language::Java.is_p1());

    assert!(!Language::Python.is_p1());
    assert!(!Language::Go.is_p1());
    assert!(!Language::C.is_p1());
}

#[test]
fn test_language_priority_completeness() {
    // Every language should be either P0, P1, or neither (P2)
    for lang in Language::all() {
        let is_p0 = lang.is_p0();
        let is_p1 = lang.is_p1();

        // P0 and P1 are mutually exclusive
        assert!(!(is_p0 && is_p1), "Language {:?} is both P0 and P1", lang);
    }
}

#[test]
fn test_language_serde_roundtrip() {
    for lang in Language::all() {
        let json = serde_json::to_string(lang).unwrap();
        let parsed: Language = serde_json::from_str(&json).unwrap();
        assert_eq!(*lang, parsed, "Serde roundtrip failed for {:?}", lang);
    }
}

#[test]
fn test_language_serde_format() {
    // Verify lowercase serialization
    assert_eq!(
        serde_json::to_string(&Language::Python).unwrap(),
        "\"python\""
    );
    assert_eq!(
        serde_json::to_string(&Language::TypeScript).unwrap(),
        "\"typescript\""
    );
    assert_eq!(
        serde_json::to_string(&Language::CSharp).unwrap(),
        "\"csharp\""
    );
}

// =============================================================================
// File System Types Tests
// =============================================================================

#[test]
fn test_node_type_variants() {
    // Verify NodeType enum variants exist and are serializable
    let dir = NodeType::Dir;
    let file = NodeType::File;

    // Test serde
    assert_eq!(serde_json::to_string(&dir).unwrap(), "\"dir\"");
    assert_eq!(serde_json::to_string(&file).unwrap(), "\"file\"");
}

#[test]
fn test_file_tree_file_creation() {
    let path = PathBuf::from("/test/file.rs");
    let tree = FileTree::file("file.rs", path.clone());

    assert_eq!(tree.name, "file.rs");
    assert_eq!(tree.node_type, NodeType::File);
    assert_eq!(tree.path, Some(path));
    assert!(tree.children.is_empty());
}

#[test]
fn test_file_tree_dir_creation() {
    let children = vec![
        FileTree::file("a.rs", PathBuf::from("/test/a.rs")),
        FileTree::file("b.rs", PathBuf::from("/test/b.rs")),
    ];
    let tree = FileTree::dir("src", children.clone());

    assert_eq!(tree.name, "src");
    assert_eq!(tree.node_type, NodeType::Dir);
    assert_eq!(tree.path, None);
    assert_eq!(tree.children.len(), 2);
}

#[test]
fn test_file_tree_serde() {
    let tree = FileTree::dir(
        "project",
        vec![FileTree::dir(
            "src",
            vec![FileTree::file(
                "main.rs",
                PathBuf::from("/project/src/main.rs"),
            )],
        )],
    );

    let json = serde_json::to_string_pretty(&tree).unwrap();
    let parsed: FileTree = serde_json::from_str(&json).unwrap();

    assert_eq!(tree.name, parsed.name);
    assert_eq!(tree.node_type, parsed.node_type);
    assert_eq!(tree.children.len(), parsed.children.len());
}

#[test]
fn test_file_entry_creation() {
    let entry = FileEntry {
        path: PathBuf::from("/test/file.py"),
        language: Some(Language::Python),
        size_bytes: 1024,
    };

    assert_eq!(entry.path, PathBuf::from("/test/file.py"));
    assert_eq!(entry.language, Some(Language::Python));
    assert_eq!(entry.size_bytes, 1024);
}

#[test]
fn test_file_entry_serde() {
    let entry = FileEntry {
        path: PathBuf::from("/test/file.py"),
        language: Some(Language::Python),
        size_bytes: 1024,
    };

    let json = serde_json::to_string(&entry).unwrap();
    let parsed: FileEntry = serde_json::from_str(&json).unwrap();

    assert_eq!(entry.path, parsed.path);
    assert_eq!(entry.language, parsed.language);
    assert_eq!(entry.size_bytes, parsed.size_bytes);
}

#[test]
fn test_ignore_spec_creation() {
    let patterns = vec!["*.pyc".to_string(), "__pycache__/".to_string()];
    let spec = IgnoreSpec::new(patterns.clone());

    assert_eq!(spec.patterns, patterns);
}

#[test]
fn test_ignore_spec_default() {
    let spec: IgnoreSpec = Default::default();
    assert!(spec.patterns.is_empty());
}

#[test]
fn test_ignore_spec_from_file_stub() {
    // TODO: This is currently a stub, returns default
    let result = IgnoreSpec::from_file(std::path::Path::new("/nonexistent/.gitignore"));
    assert!(result.is_ok());
}

#[test]
fn test_ignore_spec_is_ignored_stub() {
    // TODO: This is currently a stub, always returns false
    let spec = IgnoreSpec::new(vec!["*.pyc".to_string()]);
    assert!(!spec.is_ignored(std::path::Path::new("test.pyc")));
}

// =============================================================================
// AST Types Tests
// =============================================================================

#[test]
fn test_code_structure_creation() {
    let structure = CodeStructure {
        root: PathBuf::from("/project"),
        language: Language::Python,
        files: vec![],
    };

    assert_eq!(structure.root, PathBuf::from("/project"));
    assert_eq!(structure.language, Language::Python);
}

#[test]
fn test_file_structure_creation() {
    let file_struct = FileStructure {
        path: PathBuf::from("/test.py"),
        functions: vec!["func1".to_string(), "func2".to_string()],
        classes: vec!["Class1".to_string()],
        methods: vec!["method1".to_string()],
        imports: vec![],
        definitions: vec![],
    };

    assert_eq!(file_struct.path, PathBuf::from("/test.py"));
    assert_eq!(file_struct.functions.len(), 2);
    assert_eq!(file_struct.classes.len(), 1);
}

#[test]
fn test_import_info_creation() {
    let import = ImportInfo {
        module: "os".to_string(),
        names: vec!["path".to_string()],
        is_from: true,
        alias: Some("p".to_string()),
    };

    assert_eq!(import.module, "os");
    assert_eq!(import.names, vec!["path"]);
    assert!(import.is_from);
    assert_eq!(import.alias, Some("p".to_string()));
}

#[test]
fn test_import_info_serde() {
    let import = ImportInfo {
        module: "os".to_string(),
        names: vec!["path".to_string()],
        is_from: true,
        alias: None,
    };

    let json = serde_json::to_string(&import).unwrap();
    let parsed: ImportInfo = serde_json::from_str(&json).unwrap();

    assert_eq!(import.module, parsed.module);
}

#[test]
fn test_function_info_creation() {
    let func = FunctionInfo {
        name: "process_data".to_string(),
        params: vec!["data".to_string(), "config".to_string()],
        return_type: Some("Result".to_string()),
        docstring: Some("Process the data.".to_string()),
        is_method: false,
        is_async: true,
        decorators: vec!["@staticmethod".to_string()],
        line_number: 42,
    };

    assert_eq!(func.name, "process_data");
    assert_eq!(func.params.len(), 2);
    assert!(func.is_async);
    assert_eq!(func.line_number, 42);
}

#[test]
fn test_class_info_creation() {
    let class = ClassInfo {
        name: "DataProcessor".to_string(),
        bases: vec!["BaseProcessor".to_string()],
        docstring: Some("A data processor class.".to_string()),
        methods: vec![],
        fields: vec![],
        decorators: vec!["@dataclass".to_string()],
        line_number: 10,
    };

    assert_eq!(class.name, "DataProcessor");
    assert_eq!(class.bases, vec!["BaseProcessor"]);
    assert_eq!(class.line_number, 10);
}

#[test]
fn test_module_info_creation() {
    let module = ModuleInfo {
        file_path: PathBuf::from("/test.py"),
        language: Language::Python,
        docstring: Some("Module doc.".to_string()),
        imports: vec![],
        functions: vec![],
        classes: vec![],
        constants: vec![],
        call_graph: IntraFileCallGraph::default(),
    };

    assert_eq!(module.file_path, PathBuf::from("/test.py"));
    assert_eq!(module.language, Language::Python);
}

#[test]
fn test_intra_file_call_graph_default() {
    let graph = IntraFileCallGraph::default();
    assert!(graph.calls.is_empty());
    assert!(graph.called_by.is_empty());
}

#[test]
fn test_intra_file_call_graph_serde() {
    let mut graph = IntraFileCallGraph::default();
    graph
        .calls
        .insert("func1".to_string(), vec!["func2".to_string()]);
    graph
        .called_by
        .insert("func2".to_string(), vec!["func1".to_string()]);

    let json = serde_json::to_string(&graph).unwrap();
    let parsed: IntraFileCallGraph = serde_json::from_str(&json).unwrap();

    assert_eq!(graph.calls.len(), parsed.calls.len());
}

// =============================================================================
// Call Graph Types Tests
// =============================================================================

#[test]
fn test_function_ref_creation() {
    let func_ref = FunctionRef::new(PathBuf::from("/test.py"), "process_data");

    assert_eq!(func_ref.file, PathBuf::from("/test.py"));
    assert_eq!(func_ref.name, "process_data");
}

#[test]
fn test_function_ref_equality() {
    let ref1 = FunctionRef::new(PathBuf::from("/test.py"), "func");
    let ref2 = FunctionRef::new(PathBuf::from("/test.py"), "func");
    let ref3 = FunctionRef::new(PathBuf::from("/other.py"), "func");
    let ref4 = FunctionRef::new(PathBuf::from("/test.py"), "other");

    assert_eq!(ref1, ref2);
    assert_ne!(ref1, ref3);
    assert_ne!(ref1, ref4);
}

#[test]
fn test_function_ref_hash() {
    use std::collections::HashSet;

    let mut set = HashSet::new();
    set.insert(FunctionRef::new(PathBuf::from("/a.py"), "func1"));
    set.insert(FunctionRef::new(PathBuf::from("/a.py"), "func1")); // Duplicate
    set.insert(FunctionRef::new(PathBuf::from("/b.py"), "func2"));

    assert_eq!(set.len(), 2);
}

#[test]
fn test_function_ref_display() {
    let func_ref = FunctionRef::new(PathBuf::from("/path/to/file.py"), "process_data");
    assert_eq!(format!("{}", func_ref), "/path/to/file.py:process_data");
}

#[test]
fn test_call_edge_creation() {
    let edge = CallEdge {
        src_file: PathBuf::from("/a.py"),
        src_func: "caller".to_string(),
        dst_file: PathBuf::from("/b.py"),
        dst_func: "callee".to_string(),
    };

    assert_eq!(edge.src_file, PathBuf::from("/a.py"));
    assert_eq!(edge.src_func, "caller");
    assert_eq!(edge.dst_file, PathBuf::from("/b.py"));
    assert_eq!(edge.dst_func, "callee");
}

#[test]
fn test_project_call_graph_basic() {
    let mut graph = ProjectCallGraph::new();
    assert!(graph.is_empty());
    assert_eq!(graph.edge_count(), 0);

    let edge = CallEdge {
        src_file: PathBuf::from("/a.py"),
        src_func: "foo".to_string(),
        dst_file: PathBuf::from("/b.py"),
        dst_func: "bar".to_string(),
    };

    graph.add_edge(edge.clone());
    assert!(!graph.is_empty());
    assert_eq!(graph.edge_count(), 1);
    assert!(graph.contains(&edge));
}

#[test]
fn test_project_call_graph_edges_iterator() {
    let mut graph = ProjectCallGraph::new();
    let edge1 = CallEdge {
        src_file: PathBuf::from("/a.py"),
        src_func: "foo".to_string(),
        dst_file: PathBuf::from("/b.py"),
        dst_func: "bar".to_string(),
    };
    let edge2 = CallEdge {
        src_file: PathBuf::from("/b.py"),
        src_func: "bar".to_string(),
        dst_file: PathBuf::from("/c.py"),
        dst_func: "baz".to_string(),
    };

    graph.add_edge(edge1.clone());
    graph.add_edge(edge2.clone());

    let edges: Vec<_> = graph.edges().collect();
    assert_eq!(edges.len(), 2);
}

#[test]
fn test_project_call_graph_duplicate_edges() {
    let mut graph = ProjectCallGraph::new();
    let edge = CallEdge {
        src_file: PathBuf::from("/a.py"),
        src_func: "foo".to_string(),
        dst_file: PathBuf::from("/b.py"),
        dst_func: "bar".to_string(),
    };

    // Add same edge twice
    graph.add_edge(edge.clone());
    graph.add_edge(edge.clone());

    // Should only have one edge (HashSet behavior)
    assert_eq!(graph.edge_count(), 1);
}

// =============================================================================
// Type-Aware Call Graph Tests
// =============================================================================

#[test]
fn test_confidence_variants() {
    let high = Confidence::High;
    let medium = Confidence::Medium;
    let low = Confidence::Low;

    // Test Display
    assert_eq!(format!("{}", high), "HIGH");
    assert_eq!(format!("{}", medium), "MEDIUM");
    assert_eq!(format!("{}", low), "LOW");
}

#[test]
fn test_confidence_default() {
    let default: Confidence = Default::default();
    assert_eq!(default, Confidence::Low);
}

#[test]
fn test_confidence_serde() {
    assert_eq!(
        serde_json::to_string(&Confidence::High).unwrap(),
        "\"high\""
    );
    assert_eq!(
        serde_json::to_string(&Confidence::Medium).unwrap(),
        "\"medium\""
    );
    assert_eq!(serde_json::to_string(&Confidence::Low).unwrap(), "\"low\"");
}

#[test]
fn test_typed_call_edge_from_call_edge() {
    let call_edge = CallEdge {
        src_file: PathBuf::from("/a.py"),
        src_func: "caller".to_string(),
        dst_file: PathBuf::from("/b.py"),
        dst_func: "callee".to_string(),
    };

    let typed = TypedCallEdge::from_call_edge(&call_edge, 42);

    assert_eq!(typed.src_file, call_edge.src_file);
    assert_eq!(typed.src_func, call_edge.src_func);
    assert_eq!(typed.dst_file, call_edge.dst_file);
    assert_eq!(typed.dst_func, call_edge.dst_func);
    assert_eq!(typed.call_site_line, 42);
    assert_eq!(typed.confidence, Confidence::Low);
    assert_eq!(typed.receiver_type, None);
}

#[test]
fn test_typed_call_edge_high_confidence() {
    let edge = TypedCallEdge::high_confidence(
        PathBuf::from("/a.py"),
        "caller".to_string(),
        PathBuf::from("/b.py"),
        "callee".to_string(),
        "User".to_string(),
        42,
    );

    assert_eq!(edge.confidence, Confidence::High);
    assert_eq!(edge.receiver_type, Some("User".to_string()));
    assert_eq!(edge.call_site_line, 42);
}

#[test]
fn test_typed_call_edge_medium_confidence() {
    let edge = TypedCallEdge::medium_confidence(
        PathBuf::from("/a.py"),
        "caller".to_string(),
        PathBuf::from("/b.py"),
        "callee".to_string(),
        "User".to_string(),
        42,
    );

    assert_eq!(edge.confidence, Confidence::Medium);
    assert_eq!(edge.receiver_type, Some("User".to_string()));
}

#[test]
fn test_typed_call_edge_to_call_edge() {
    let typed = TypedCallEdge::high_confidence(
        PathBuf::from("/a.py"),
        "caller".to_string(),
        PathBuf::from("/b.py"),
        "callee".to_string(),
        "User".to_string(),
        42,
    );

    let basic = typed.to_call_edge();

    assert_eq!(basic.src_file, typed.src_file);
    assert_eq!(basic.src_func, typed.src_func);
    assert_eq!(basic.dst_file, typed.dst_file);
    assert_eq!(basic.dst_func, typed.dst_func);
}

#[test]
fn test_type_resolution_stats_enabled() {
    let stats = TypeResolutionStats::enabled();
    assert!(stats.enabled);
    assert_eq!(stats.total_call_sites, 0);
}

#[test]
fn test_type_resolution_stats_default() {
    let stats: TypeResolutionStats = Default::default();
    assert!(!stats.enabled);
}

#[test]
fn test_type_resolution_stats_recording() {
    let mut stats = TypeResolutionStats::enabled();

    stats.record_high();
    stats.record_medium();
    stats.record_fallback();

    assert_eq!(stats.resolved_high_confidence, 1);
    assert_eq!(stats.resolved_medium_confidence, 1);
    assert_eq!(stats.fallback_used, 1);
    assert_eq!(stats.total_call_sites, 3);
}

#[test]
fn test_type_resolution_stats_resolution_rate() {
    let mut stats = TypeResolutionStats::enabled();

    assert_eq!(stats.resolution_rate(), 0.0);

    stats.record_high();
    stats.record_medium();
    stats.record_fallback();

    // 2 resolved out of 3 = 66.67%
    assert!((stats.resolution_rate() - 66.67).abs() < 0.1);
}

#[test]
fn test_type_resolution_stats_summary_disabled() {
    let stats = TypeResolutionStats::default();
    assert_eq!(stats.summary(), "Type resolution: disabled");
}

#[test]
fn test_type_resolution_stats_summary_enabled() {
    let mut stats = TypeResolutionStats::enabled();
    stats.record_high();
    stats.record_medium();

    let summary = stats.summary();
    assert!(summary.contains("Type-aware resolution"));
    assert!(summary.contains("high"));
    assert!(summary.contains("medium"));
}

// =============================================================================
// Impact Analysis Types Tests
// =============================================================================

#[test]
fn test_impact_report_creation() {
    let report = ImpactReport {
        targets: HashMap::new(),
        total_targets: 0,
        type_resolution: None,
    };

    assert!(report.targets.is_empty());
    assert_eq!(report.total_targets, 0);
}

#[test]
fn test_caller_tree_creation() {
    let tree = CallerTree {
        function: "main".to_string(),
        file: PathBuf::from("/app.py"),
        caller_count: 0,
        callers: vec![],
        truncated: false,
        note: None,
        confidence: Some(Confidence::High),
        receiver_type: Some("App".to_string()),
    };

    assert_eq!(tree.function, "main");
    assert!(!tree.truncated);
}

// =============================================================================
// Dead Code Types Tests
// =============================================================================

#[test]
fn test_dead_code_report_creation() {
    let report = DeadCodeReport {
        dead_functions: vec![],
        possibly_dead: vec![],
        by_file: HashMap::new(),
        total_dead: 0,
        total_possibly_dead: 0,
        total_functions: 100,
        dead_percentage: 0.0,
    };

    assert_eq!(report.total_functions, 100);
    assert_eq!(report.dead_percentage, 0.0);
}

// =============================================================================
// Importers Types Tests
// =============================================================================

#[test]
fn test_importers_report_creation() {
    let report = ImportersReport {
        module: "os".to_string(),
        importers: vec![],
        total: 0,
    };

    assert_eq!(report.module, "os");
}

#[test]
fn test_importer_info_creation() {
    let info = ImporterInfo {
        file: PathBuf::from("/test.py"),
        line: 10,
        import_statement: "import os".to_string(),
    };

    assert_eq!(info.line, 10);
    assert_eq!(info.import_statement, "import os");
}

// =============================================================================
// Architecture Types Tests
// =============================================================================

#[test]
fn test_architecture_report_creation() {
    let report = ArchitectureReport {
        entry_layer: vec![],
        middle_layer: vec![],
        leaf_layer: vec![],
        directories: HashMap::new(),
        circular_dependencies: vec![],
        inferred_layers: HashMap::new(),
    };

    assert!(report.entry_layer.is_empty());
    assert!(report.circular_dependencies.is_empty());
}

#[test]
fn test_dir_stats_creation() {
    let stats = DirStats {
        functions: vec!["func1".to_string()],
        calls_out: 5,
        calls_in: 3,
    };

    assert_eq!(stats.calls_out, 5);
    assert_eq!(stats.calls_in, 3);
}

#[test]
fn test_circular_dep_creation() {
    let dep = CircularDep {
        a: PathBuf::from("/dir1"),
        b: PathBuf::from("/dir2"),
    };

    assert_eq!(dep.a, PathBuf::from("/dir1"));
    assert_eq!(dep.b, PathBuf::from("/dir2"));
}

#[test]
fn test_layer_type_variants() {
    let entry = LayerType::Entry;
    let service = LayerType::Service;
    let utility = LayerType::Utility;
    let dynamic = LayerType::DynamicDispatch;

    // Just verify they exist and can be compared
    assert_ne!(entry, service);
    assert_ne!(utility, dynamic);
}

// =============================================================================
// CFG Types Tests
// =============================================================================

#[test]
fn test_block_type_variants() {
    // Test all variants exist
    let _entry = BlockType::Entry;
    let _branch = BlockType::Branch;
    let _loop_header = BlockType::LoopHeader;
    let _loop_body = BlockType::LoopBody;
    let _return = BlockType::Return;
    let _exit = BlockType::Exit;
    let _body = BlockType::Body;

    // Test serde
    assert_eq!(
        serde_json::to_string(&BlockType::Entry).unwrap(),
        "\"entry\""
    );
    assert_eq!(
        serde_json::to_string(&BlockType::Branch).unwrap(),
        "\"branch\""
    );
    assert_eq!(
        serde_json::to_string(&BlockType::LoopHeader).unwrap(),
        "\"loop_header\""
    );
}

#[test]
fn test_edge_type_variants() {
    let _true = EdgeType::True;
    let _false = EdgeType::False;
    let _unconditional = EdgeType::Unconditional;
    let _back_edge = EdgeType::BackEdge;
    let _break = EdgeType::Break;
    let _continue = EdgeType::Continue;

    // Test serde (snake_case)
    assert_eq!(serde_json::to_string(&EdgeType::True).unwrap(), "\"true\"");
    assert_eq!(
        serde_json::to_string(&EdgeType::BackEdge).unwrap(),
        "\"back_edge\""
    );
}

#[test]
fn test_cfg_block_creation() {
    let block = CfgBlock {
        id: 0,
        block_type: BlockType::Entry,
        lines: (1, 10),
        calls: vec!["helper".to_string()],
    };

    assert_eq!(block.id, 0);
    assert_eq!(block.lines, (1, 10));
}

#[test]
fn test_cfg_edge_creation() {
    let edge = CfgEdge {
        from: 0,
        to: 1,
        edge_type: EdgeType::True,
        condition: Some("x > 0".to_string()),
    };

    assert_eq!(edge.from, 0);
    assert_eq!(edge.to, 1);
    assert_eq!(edge.condition, Some("x > 0".to_string()));
}

#[test]
fn test_cfg_info_creation() {
    let cfg = CfgInfo {
        function: "main".to_string(),
        blocks: vec![],
        edges: vec![],
        entry_block: 0,
        exit_blocks: vec![1],
        cyclomatic_complexity: 2,
        nested_functions: HashMap::new(),
    };

    assert_eq!(cfg.function, "main");
    assert_eq!(cfg.cyclomatic_complexity, 2);
}

#[test]
fn test_complexity_metrics_creation() {
    let metrics = ComplexityMetrics {
        function: "process".to_string(),
        cyclomatic: 5,
        cognitive: 3,
        nesting_depth: 2,
        lines_of_code: 50,
    };

    assert_eq!(metrics.cyclomatic, 5);
    assert_eq!(metrics.cognitive, 3);
    assert_eq!(metrics.nesting_depth, 2);
}

// =============================================================================
// DFG Types Tests
// =============================================================================

#[test]
fn test_ref_type_variants() {
    let def = RefType::Definition;
    let update = RefType::Update;
    let use_ref = RefType::Use;

    // Test serde (lowercase)
    assert_eq!(serde_json::to_string(&def).unwrap(), "\"definition\"");
    assert_eq!(serde_json::to_string(&update).unwrap(), "\"update\"");
    assert_eq!(serde_json::to_string(&use_ref).unwrap(), "\"use\"");
}

#[test]
fn test_var_ref_creation() {
    let var_ref = VarRef {
        name: "x".to_string(),
        ref_type: RefType::Definition,
        line: 10,
        column: 5,
        context: Some(VarRefContext::AugmentedAssignment),
        group_id: None,
    };

    assert_eq!(var_ref.name, "x");
    assert_eq!(var_ref.line, 10);
}

#[test]
fn test_var_ref_context_variants() {
    // Python-specific
    let _aug = VarRefContext::AugmentedAssignment;
    let _multi = VarRefContext::MultipleAssignment;
    let _walrus = VarRefContext::WalrusOperator;
    let _comp = VarRefContext::ComprehensionScope;
    let _match_bind = VarRefContext::MatchBinding;
    let _global = VarRefContext::GlobalNonlocal;

    // TypeScript/JavaScript-specific
    let _destructure = VarRefContext::Destructuring;
    let _closure = VarRefContext::ClosureCapture;
    let _optional = VarRefContext::OptionalChain;

    // Go-specific
    let _short = VarRefContext::ShortDeclaration;
    let _multi_ret = VarRefContext::MultipleReturn;
    let _blank = VarRefContext::BlankIdentifier;
    let _defer = VarRefContext::DeferCapture;

    // Rust-specific
    let _shadow = VarRefContext::Shadowing;
    let _pattern = VarRefContext::PatternBinding;
    let _move = VarRefContext::OwnershipMove;
    let _match_arm = VarRefContext::MatchArmBinding;

    // Test serde (snake_case)
    assert_eq!(
        serde_json::to_string(&VarRefContext::AugmentedAssignment).unwrap(),
        "\"augmented_assignment\""
    );
    assert_eq!(
        serde_json::to_string(&VarRefContext::WalrusOperator).unwrap(),
        "\"walrus_operator\""
    );
}

#[test]
fn test_dataflow_edge_creation() {
    let def_ref = VarRef {
        name: "x".to_string(),
        ref_type: RefType::Definition,
        line: 10,
        column: 5,
        context: None,
        group_id: None,
    };

    let use_ref = VarRef {
        name: "x".to_string(),
        ref_type: RefType::Use,
        line: 15,
        column: 10,
        context: None,
        group_id: None,
    };

    let edge = DataflowEdge {
        var: "x".to_string(),
        def_line: 10,
        use_line: 15,
        def_ref,
        use_ref,
    };

    assert_eq!(edge.var, "x");
    assert_eq!(edge.def_line, 10);
    assert_eq!(edge.use_line, 15);
}

#[test]
fn test_dfg_info_creation() {
    let dfg = DfgInfo {
        function: "main".to_string(),
        refs: vec![],
        edges: vec![],
        variables: vec!["x".to_string(), "y".to_string()],
    };

    assert_eq!(dfg.function, "main");
    assert_eq!(dfg.variables.len(), 2);
}

// =============================================================================
// PDG Types Tests
// =============================================================================

#[test]
fn test_dependence_type_variants() {
    let control = DependenceType::Control;
    let data = DependenceType::Data;

    assert_eq!(serde_json::to_string(&control).unwrap(), "\"control\"");
    assert_eq!(serde_json::to_string(&data).unwrap(), "\"data\"");
}

#[test]
fn test_slice_direction_from_str() {
    assert_eq!(
        SliceDirection::from_str("backward").unwrap(),
        SliceDirection::Backward
    );
    assert_eq!(
        SliceDirection::from_str("forward").unwrap(),
        SliceDirection::Forward
    );
    assert_eq!(
        SliceDirection::from_str("back").unwrap(),
        SliceDirection::Backward
    );
    assert_eq!(
        SliceDirection::from_str("fwd").unwrap(),
        SliceDirection::Forward
    );
    assert_eq!(
        SliceDirection::from_str("b").unwrap(),
        SliceDirection::Backward
    );
    assert_eq!(
        SliceDirection::from_str("f").unwrap(),
        SliceDirection::Forward
    );

    // Case insensitive
    assert_eq!(
        SliceDirection::from_str("BACKWARD").unwrap(),
        SliceDirection::Backward
    );
    assert_eq!(
        SliceDirection::from_str("Forward").unwrap(),
        SliceDirection::Forward
    );
}

#[test]
fn test_slice_direction_from_str_invalid() {
    assert!(SliceDirection::from_str("invalid").is_err());
    assert!(SliceDirection::from_str("").is_err());
    assert!(SliceDirection::from_str("up").is_err());
}

#[test]
fn test_slice_direction_serde() {
    assert_eq!(
        serde_json::to_string(&SliceDirection::Backward).unwrap(),
        "\"backward\""
    );
    assert_eq!(
        serde_json::to_string(&SliceDirection::Forward).unwrap(),
        "\"forward\""
    );
}

#[test]
fn test_thin_slice_result_creation() {
    let mut lines = HashSet::new();
    lines.insert(1);
    lines.insert(2);
    lines.insert(3);

    let mut full_lines = HashSet::new();
    full_lines.insert(1);
    full_lines.insert(2);
    full_lines.insert(3);
    full_lines.insert(4);
    full_lines.insert(5);

    let result = ThinSliceResult {
        lines,
        full_slice_lines: full_lines,
        reduction_pct: 40.0,
    };

    assert_eq!(result.reduction_pct, 40.0);
    assert_eq!(result.lines.len(), 3);
    assert_eq!(result.full_slice_lines.len(), 5);
}

#[test]
fn test_pdg_node_creation() {
    let node = PdgNode {
        id: 0,
        node_type: "statement".to_string(),
        lines: (10, 15),
        definitions: vec!["x".to_string()],
        uses: vec!["y".to_string()],
    };

    assert_eq!(node.id, 0);
    assert_eq!(node.node_type, "statement");
}

#[test]
fn test_pdg_edge_creation() {
    let edge = PdgEdge {
        source_id: 0,
        target_id: 1,
        dep_type: DependenceType::Data,
        label: "data-dep".to_string(),
    };

    assert_eq!(edge.source_id, 0);
    assert_eq!(edge.target_id, 1);
    assert_eq!(edge.dep_type, DependenceType::Data);
}

#[test]
fn test_pdg_info_creation() {
    let pdg = PdgInfo {
        function: "main".to_string(),
        cfg: CfgInfo {
            function: "main".to_string(),
            blocks: vec![],
            edges: vec![],
            entry_block: 0,
            exit_blocks: vec![],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        },
        dfg: DfgInfo {
            function: "main".to_string(),
            refs: vec![],
            edges: vec![],
            variables: vec![],
        },
        nodes: vec![],
        edges: vec![],
    };

    assert_eq!(pdg.function, "main");
}

// =============================================================================
// Search Types Tests
// =============================================================================

#[test]
fn test_search_match_creation() {
    let match_result = SearchMatch {
        file: PathBuf::from("/test.py"),
        line: 42,
        content: "def process_data():".to_string(),
        context: Some(vec!["# Comment".to_string()]),
    };

    assert_eq!(match_result.line, 42);
    assert_eq!(match_result.content, "def process_data():");
}

#[test]
fn test_bm25_result_creation() {
    let result = Bm25Result {
        file_path: PathBuf::from("/test.py"),
        score: 1.5,
        line_start: 10,
        line_end: 20,
        snippet: "def foo():".to_string(),
        matched_terms: vec!["def".to_string(), "foo".to_string()],
    };

    assert!(result.score > 0.0);
    assert_eq!(result.line_start, 10);
}

#[test]
fn test_hybrid_result_creation() {
    let result = HybridResult {
        file_path: PathBuf::from("/test.py"),
        rrf_score: 0.5,
        bm25_rank: Some(1),
        dense_rank: Some(2),
        bm25_score: Some(1.2),
        dense_score: Some(0.8),
        snippet: "code".to_string(),
        matched_terms: vec![],
    };

    assert_eq!(result.rrf_score, 0.5);
    assert_eq!(result.bm25_rank, Some(1));
}

#[test]
fn test_hybrid_search_report_creation() {
    let report = HybridSearchReport {
        results: vec![],
        query: "test".to_string(),
        total_candidates: 100,
        bm25_only: 10,
        dense_only: 5,
        overlap: 20,
        fallback_mode: None,
    };

    assert_eq!(report.query, "test");
    assert_eq!(report.total_candidates, 100);
}

// =============================================================================
// Context Types Tests
// =============================================================================

#[test]
fn test_function_context_creation() {
    let ctx = FunctionContext {
        name: "main".to_string(),
        file: PathBuf::from("/app.py"),
        line: 10,
        signature: "def main() -> None".to_string(),
        docstring: Some("Entry point.".to_string()),
        calls: vec!["helper".to_string()],
        blocks: Some(3),
        cyclomatic: Some(2),
    };

    assert_eq!(ctx.name, "main");
    assert_eq!(ctx.line, 10);
}

#[test]
fn test_relevant_context_creation() {
    let ctx = RelevantContext {
        entry_point: "main".to_string(),
        depth: 2,
        functions: vec![],
    };

    assert_eq!(ctx.entry_point, "main");
    assert_eq!(ctx.depth, 2);
}

#[test]
fn test_relevant_context_to_llm_string() {
    let ctx = RelevantContext {
        entry_point: "process".to_string(),
        depth: 1,
        functions: vec![FunctionContext {
            name: "process".to_string(),
            file: PathBuf::from("/app.py"),
            line: 10,
            signature: "def process(data)".to_string(),
            docstring: None,
            calls: vec![],
            blocks: None,
            cyclomatic: None,
        }],
    };

    let output = ctx.to_llm_string();
    assert!(output.contains("Context for: process"));
    assert!(output.contains("def process(data)"));
    assert!(output.contains("/app.py:10"));
}

// =============================================================================
// Change Impact Types Tests
// =============================================================================

#[test]
fn test_change_impact_report_creation() {
    let report = ChangeImpactReport {
        changed_files: vec![PathBuf::from("/a.py")],
        affected_tests: vec![PathBuf::from("/test_a.py")],
        affected_functions: vec![FunctionRef::new(PathBuf::from("/a.py"), "func")],
        detection_method: "call_graph".to_string(),
    };

    assert_eq!(report.detection_method, "call_graph");
    assert_eq!(report.changed_files.len(), 1);
}

// =============================================================================
// Quality Types Tests
// =============================================================================

#[test]
fn test_threshold_preset_variants() {
    let _strict = ThresholdPreset::Strict;
    let _default = ThresholdPreset::Default;
    let _relaxed = ThresholdPreset::Relaxed;

    // Test default
    let preset: ThresholdPreset = Default::default();
    assert_eq!(preset, ThresholdPreset::Default);
}

#[test]
fn test_smell_type_variants() {
    let types = [SmellType::GodClass,
        SmellType::LongMethod,
        SmellType::FeatureEnvy,
        SmellType::DataClumps,
        SmellType::LongParameterList];

    assert_eq!(types.len(), 5);
}

#[test]
fn test_smell_finding_creation() {
    let finding = SmellFinding {
        file: PathBuf::from("/test.py"),
        line: 42,
        smell_type: SmellType::LongMethod,
        description: "Method is too long".to_string(),
        suggestion: Some("Refactor into smaller methods".to_string()),
    };

    assert_eq!(finding.smell_type, SmellType::LongMethod);
    assert_eq!(finding.line, 42);
}

#[test]
fn test_smells_report_creation() {
    let report = SmellsReport {
        smells: vec![],
        files_analyzed: 10,
        total_smells: 0,
    };

    assert_eq!(report.files_analyzed, 10);
}

#[test]
fn test_halstead_metrics_creation() {
    let metrics = HalsteadMetrics {
        vocabulary: 20,
        length: 100,
        volume: 500.0,
        difficulty: 10.0,
        effort: 5000.0,
    };

    assert_eq!(metrics.vocabulary, 20);
    assert_eq!(metrics.effort, 5000.0);
}

#[test]
fn test_file_mi_creation() {
    let file_mi = FileMI {
        path: PathBuf::from("/test.py"),
        mi: 85.5,
        grade: 'A',
        halstead: None,
    };

    assert_eq!(file_mi.mi, 85.5);
    assert_eq!(file_mi.grade, 'A');
}

#[test]
fn test_mi_summary_creation() {
    let summary = MISummary {
        average_mi: 80.0,
        min_mi: 60.0,
        max_mi: 95.0,
        files_analyzed: 10,
    };

    assert_eq!(summary.average_mi, 80.0);
    assert_eq!(summary.files_analyzed, 10);
}

#[test]
fn test_maintainability_report_creation() {
    let report = MaintainabilityReport {
        files: vec![],
        summary: MISummary {
            average_mi: 80.0,
            min_mi: 60.0,
            max_mi: 95.0,
            files_analyzed: 0,
        },
    };

    assert_eq!(report.summary.average_mi, 80.0);
}

// =============================================================================
// Security Types Tests
// =============================================================================

#[test]
fn test_severity_ordering() {
    assert!(Severity::Low < Severity::Medium);
    assert!(Severity::Medium < Severity::High);
    assert!(Severity::High < Severity::Critical);
}

#[test]
fn test_severity_serde() {
    // Just verify it serializes (order is preserved via derive)
    let sev = Severity::High;
    let json = serde_json::to_string(&sev).unwrap();
    let parsed: Severity = serde_json::from_str(&json).unwrap();
    assert_eq!(sev, parsed);
}

#[test]
fn test_secret_finding_creation() {
    let finding = SecretFinding {
        file: PathBuf::from("/config.py"),
        line: 10,
        pattern: "AWS_KEY".to_string(),
        severity: Severity::Critical,
        masked_value: "AKIA****".to_string(),
    };

    assert_eq!(finding.pattern, "AWS_KEY");
    assert_eq!(finding.severity, Severity::Critical);
}

#[test]
fn test_secrets_summary_creation() {
    let mut by_severity = HashMap::new();
    by_severity.insert("Critical".to_string(), 1);

    let summary = SecretsSummary {
        total_findings: 1,
        by_severity,
    };

    assert_eq!(summary.total_findings, 1);
}

#[test]
fn test_vuln_type_variants() {
    let types = [VulnType::SqlInjection,
        VulnType::Xss,
        VulnType::CommandInjection,
        VulnType::PathTraversal,
        VulnType::Ssrf,
        VulnType::Deserialization];

    assert_eq!(types.len(), 6);
}

#[test]
fn test_vuln_finding_creation() {
    let finding = VulnFinding {
        file: PathBuf::from("/app.py"),
        line: 25,
        vuln_type: VulnType::SqlInjection,
        severity: Severity::High,
        description: "Unsanitized user input".to_string(),
        source: Some("request.args".to_string()),
        sink: Some("cursor.execute".to_string()),
    };

    assert_eq!(finding.vuln_type, VulnType::SqlInjection);
    assert_eq!(finding.severity, Severity::High);
}

#[test]
fn test_vuln_summary_creation() {
    let summary = VulnSummary {
        total_findings: 5,
        by_type: HashMap::new(),
        by_severity: HashMap::new(),
    };

    assert_eq!(summary.total_findings, 5);
}

// =============================================================================
// Workspace Config Tests
// =============================================================================

#[test]
fn test_workspace_config_default() {
    let config = WorkspaceConfig::default();
    assert!(config.roots.is_empty());
}

// =============================================================================
// Integration/Serde Tests
// =============================================================================

#[test]
fn test_full_serde_roundtrip_complex_types() {
    // Test a complex nested structure
    let func = FunctionInfo {
        name: "process".to_string(),
        params: vec!["data".to_string()],
        return_type: Some("Result".to_string()),
        docstring: Some("Process data".to_string()),
        is_method: false,
        is_async: true,
        decorators: vec![],
        line_number: 42,
    };

    let json = serde_json::to_string_pretty(&func).unwrap();
    let parsed: FunctionInfo = serde_json::from_str(&json).unwrap();

    assert_eq!(func.name, parsed.name);
    assert_eq!(func.params, parsed.params);
    assert_eq!(func.is_async, parsed.is_async);
}

#[test]
fn test_hashmap_key_types() {
    // Verify types that should work as HashMap keys
    let mut map: HashMap<FunctionRef, String> = HashMap::new();
    map.insert(
        FunctionRef::new(PathBuf::from("/a.py"), "func"),
        "value".to_string(),
    );

    assert_eq!(map.len(), 1);
}

#[test]
fn test_skip_serializing_if_behavior() {
    // Test that Option::None fields are skipped
    let import = ImportInfo {
        module: "os".to_string(),
        names: vec![],  // Empty vec should be skipped
        is_from: false, // Default should be skipped
        alias: None,    // None should be skipped
    };

    let json = serde_json::to_string(&import).unwrap();

    // Should not contain skipped fields
    assert!(json.contains("module"));
    // Empty vec should be skipped
    assert!(!json.contains("names"));
}

#[test]
fn test_language_from_directory_real() {
    use std::fs;

    let tmp = std::env::temp_dir().join("tldr_test_lang_dir");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    // Create mixed files
    fs::write(tmp.join("main.py"), "").unwrap();
    fs::write(tmp.join("lib.py"), "").unwrap();
    fs::write(tmp.join("app.js"), "").unwrap();

    let detected = Language::from_directory(&tmp);
    // Python has more files (2 vs 1)
    assert_eq!(detected, Some(Language::Python));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_language_from_directory_skips_hidden() {
    use std::fs;

    let tmp = std::env::temp_dir().join("tldr_test_hidden");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::create_dir_all(tmp.join(".git")).unwrap();
    fs::create_dir_all(tmp.join(".tldr")).unwrap();

    // Create files in hidden dirs (should be skipped)
    fs::write(tmp.join(".git/hooks.py"), "").unwrap();
    fs::write(tmp.join(".tldr/cache.rs"), "").unwrap();

    // Create non-hidden file
    fs::write(tmp.join("main.go"), "").unwrap();

    let detected = Language::from_directory(&tmp);
    // Should detect Go (hidden dirs skipped)
    assert_eq!(detected, Some(Language::Go));

    let _ = fs::remove_dir_all(&tmp);
}
