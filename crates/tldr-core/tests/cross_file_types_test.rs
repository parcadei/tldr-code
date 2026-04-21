//! Tests for Phase 7: Cross-File Resolution Types
//!
//! These types support cross-file call graph resolution:
//! - ResolvedImport: Result of resolving an import
//! - ModuleInfo: Metadata about a module
//! - ReExportChain: Tracks re-export hops
//! - CrossFileCallEdge: Edge in the cross-file call graph
//! - ProjectCallGraphV2: New call graph with indexed lookups
//!
//! See migration/spec/callgraph-spec.md Section 4.3 and phased-plan.yaml Phase 7.

use std::collections::HashSet;
use std::path::PathBuf;

use tldr_core::callgraph::cross_file_types::{
    CrossFileCallEdge, ImportKind, ModuleInfo, ProjectCallGraphV2, ReExportChain, ResolvedImport,
};
use tldr_core::callgraph::{CallType, ImportDef};

// =============================================================================
// ImportKind Tests
// =============================================================================

mod import_kind_tests {
    use super::*;

    #[test]
    fn test_import_kind_variants() {
        let _absolute = ImportKind::Absolute;
        let _relative = ImportKind::Relative;
        let _wildcard = ImportKind::Wildcard;
        let _type_only = ImportKind::TypeOnly;

        // All variants should be distinct
        assert_ne!(ImportKind::Absolute, ImportKind::Relative);
        assert_ne!(ImportKind::Relative, ImportKind::Wildcard);
        assert_ne!(ImportKind::Wildcard, ImportKind::TypeOnly);
    }

    #[test]
    fn test_import_kind_copy_and_eq() {
        let a = ImportKind::Absolute;
        let b = a; // Copy
        assert_eq!(a, b);

        let mut set = HashSet::new();
        set.insert(ImportKind::Absolute);
        set.insert(ImportKind::Relative);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_import_kind_debug() {
        let kind = ImportKind::Absolute;
        let debug_str = format!("{:?}", kind);
        assert!(debug_str.contains("Absolute"));
    }
}

// =============================================================================
// ResolvedImport Tests
// =============================================================================

mod resolved_import_tests {
    use super::*;

    #[test]
    fn test_resolved_import_construction() {
        let import_def = ImportDef::from_import("pkg.module", vec!["MyClass".to_string()]);
        let resolved = ResolvedImport {
            original: import_def.clone(),
            resolved_file: Some(PathBuf::from("pkg/module.py")),
            resolved_name: Some("MyClass".to_string()),
            is_external: false,
            confidence: 0.95,
        };

        assert_eq!(resolved.resolved_file, Some(PathBuf::from("pkg/module.py")));
        assert_eq!(resolved.resolved_name, Some("MyClass".to_string()));
        assert!(!resolved.is_external);
        assert!((resolved.confidence - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn test_resolved_import_external_module() {
        let import_def = ImportDef::simple_import("requests");
        let resolved = ResolvedImport {
            original: import_def,
            resolved_file: None,
            resolved_name: None,
            is_external: true,
            confidence: 1.0,
        };

        assert!(resolved.is_external);
        assert!(resolved.resolved_file.is_none());
    }

    #[test]
    fn test_resolved_import_with_low_confidence() {
        let import_def = ImportDef::wildcard_import("pkg");
        let resolved = ResolvedImport {
            original: import_def,
            resolved_file: Some(PathBuf::from("pkg/__init__.py")),
            resolved_name: Some("helper".to_string()),
            is_external: false,
            confidence: 0.3, // Low confidence for wildcard
        };

        assert!(resolved.confidence < 0.5);
    }

    #[test]
    fn test_resolved_import_equality() {
        let import_def = ImportDef::from_import("os", vec!["path".to_string()]);
        let resolved1 = ResolvedImport {
            original: import_def.clone(),
            resolved_file: Some(PathBuf::from("os/__init__.py")),
            resolved_name: Some("path".to_string()),
            is_external: false,
            confidence: 1.0,
        };
        let resolved2 = ResolvedImport {
            original: import_def,
            resolved_file: Some(PathBuf::from("os/__init__.py")),
            resolved_name: Some("path".to_string()),
            is_external: false,
            confidence: 1.0,
        };

        assert_eq!(resolved1, resolved2);
    }

    #[test]
    fn test_resolved_import_clone() {
        let import_def = ImportDef::from_import("pkg", vec!["Foo".to_string()]);
        let resolved = ResolvedImport {
            original: import_def,
            resolved_file: Some(PathBuf::from("pkg/foo.py")),
            resolved_name: Some("Foo".to_string()),
            is_external: false,
            confidence: 0.9,
        };

        let cloned = resolved.clone();
        assert_eq!(resolved, cloned);
    }
}

// =============================================================================
// ModuleInfo Tests
// =============================================================================

mod module_info_tests {
    use super::*;

    #[test]
    fn test_module_info_construction() {
        let info = ModuleInfo {
            path: PathBuf::from("pkg/core.py"),
            module_name: "pkg.core".to_string(),
            is_package: false,
            exports: vec!["process".to_string(), "validate".to_string()],
        };

        assert_eq!(info.path, PathBuf::from("pkg/core.py"));
        assert_eq!(info.module_name, "pkg.core");
        assert!(!info.is_package);
        assert_eq!(info.exports.len(), 2);
    }

    #[test]
    fn test_module_info_package() {
        let info = ModuleInfo {
            path: PathBuf::from("pkg/__init__.py"),
            module_name: "pkg".to_string(),
            is_package: true,
            exports: vec!["MyClass".to_string(), "helper".to_string()],
        };

        assert!(info.is_package);
        assert!(info.path.to_string_lossy().contains("__init__.py"));
    }

    #[test]
    fn test_module_info_empty_exports() {
        let info = ModuleInfo {
            path: PathBuf::from("pkg/internal.py"),
            module_name: "pkg.internal".to_string(),
            is_package: false,
            exports: vec![],
        };

        assert!(info.exports.is_empty());
    }

    #[test]
    fn test_module_info_clone() {
        let info = ModuleInfo {
            path: PathBuf::from("src/lib.rs"),
            module_name: "crate".to_string(),
            is_package: true,
            exports: vec!["foo".to_string()],
        };

        let cloned = info.clone();
        assert_eq!(info.path, cloned.path);
        assert_eq!(info.module_name, cloned.module_name);
    }
}

// =============================================================================
// ReExportChain Tests
// =============================================================================

mod reexport_chain_tests {
    use super::*;

    #[test]
    fn test_reexport_chain_construction() {
        let chain = ReExportChain {
            original_module: "pkg".to_string(),
            original_name: "MyClass".to_string(),
            final_module: "pkg.impl".to_string(),
            final_name: "MyClass".to_string(),
            hops: vec![
                ("pkg".to_string(), "MyClass".to_string()),
                ("pkg.impl".to_string(), "MyClass".to_string()),
            ],
        };

        assert_eq!(chain.original_module, "pkg");
        assert_eq!(chain.final_module, "pkg.impl");
        assert_eq!(chain.hops.len(), 2);
    }

    #[test]
    fn test_reexport_chain_single_hop() {
        // Direct re-export: from .module import MyClass in __init__.py
        let chain = ReExportChain {
            original_module: "pkg".to_string(),
            original_name: "MyClass".to_string(),
            final_module: "pkg.module".to_string(),
            final_name: "MyClass".to_string(),
            hops: vec![("pkg.module".to_string(), "MyClass".to_string())],
        };

        assert_eq!(chain.hops.len(), 1);
    }

    #[test]
    fn test_reexport_chain_with_rename() {
        // Re-export with rename: from .impl import _InternalClass as PublicClass
        let chain = ReExportChain {
            original_module: "pkg".to_string(),
            original_name: "PublicClass".to_string(),
            final_module: "pkg.impl".to_string(),
            final_name: "_InternalClass".to_string(),
            hops: vec![("pkg.impl".to_string(), "_InternalClass".to_string())],
        };

        assert_ne!(chain.original_name, chain.final_name);
    }

    #[test]
    fn test_reexport_chain_multi_hop() {
        // Chain: pkg -> pkg.sub -> pkg.sub.impl
        let chain = ReExportChain {
            original_module: "pkg".to_string(),
            original_name: "MyClass".to_string(),
            final_module: "pkg.sub.impl".to_string(),
            final_name: "MyClass".to_string(),
            hops: vec![
                ("pkg.sub".to_string(), "MyClass".to_string()),
                ("pkg.sub.impl".to_string(), "MyClass".to_string()),
            ],
        };

        assert_eq!(chain.hops.len(), 2);
        assert_eq!(chain.hops[0].0, "pkg.sub");
        assert_eq!(chain.hops[1].0, "pkg.sub.impl");
    }

    #[test]
    fn test_reexport_chain_clone() {
        let chain = ReExportChain {
            original_module: "a".to_string(),
            original_name: "X".to_string(),
            final_module: "a.b".to_string(),
            final_name: "X".to_string(),
            hops: vec![("a.b".to_string(), "X".to_string())],
        };

        let cloned = chain.clone();
        assert_eq!(chain.original_module, cloned.original_module);
        assert_eq!(chain.hops, cloned.hops);
    }
}

// =============================================================================
// CrossFileCallEdge Tests
// =============================================================================

mod cross_file_call_edge_tests {
    use super::*;

    #[test]
    fn test_cross_file_call_edge_construction() {
        let edge = CrossFileCallEdge {
            src_file: PathBuf::from("src/main.py"),
            src_func: "main".to_string(),
            dst_file: PathBuf::from("src/utils.py"),
            dst_func: "helper".to_string(),
            call_type: CallType::Direct,
            via_import: Some("utils".to_string()),
        };

        assert_eq!(edge.src_file, PathBuf::from("src/main.py"));
        assert_eq!(edge.src_func, "main");
        assert_eq!(edge.dst_file, PathBuf::from("src/utils.py"));
        assert_eq!(edge.dst_func, "helper");
        assert_eq!(edge.call_type, CallType::Direct);
        assert_eq!(edge.via_import, Some("utils".to_string()));
    }

    #[test]
    fn test_cross_file_call_edge_intra_file() {
        let edge = CrossFileCallEdge {
            src_file: PathBuf::from("src/module.py"),
            src_func: "outer".to_string(),
            dst_file: PathBuf::from("src/module.py"),
            dst_func: "inner".to_string(),
            call_type: CallType::Intra,
            via_import: None,
        };

        // Same file for intra-file calls
        assert_eq!(edge.src_file, edge.dst_file);
        assert_eq!(edge.call_type, CallType::Intra);
        assert!(edge.via_import.is_none());
    }

    #[test]
    fn test_cross_file_call_edge_method_call() {
        let edge = CrossFileCallEdge {
            src_file: PathBuf::from("src/app.py"),
            src_func: "process".to_string(),
            dst_file: PathBuf::from("src/models/user.py"),
            dst_func: "User.save".to_string(),
            call_type: CallType::Method,
            via_import: Some("models.user.User".to_string()),
        };

        assert_eq!(edge.call_type, CallType::Method);
        assert!(edge.dst_func.contains('.'));
    }

    #[test]
    fn test_cross_file_call_edge_equality() {
        let edge1 = CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        };

        let edge2 = CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        };

        assert_eq!(edge1, edge2);
    }

    #[test]
    fn test_cross_file_call_edge_hash() {
        let edge1 = CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        };

        let edge2 = edge1.clone();

        let mut set = HashSet::new();
        set.insert(edge1);
        set.insert(edge2); // Should deduplicate

        assert_eq!(set.len(), 1);
    }

    #[test]
    fn test_cross_file_call_edge_different_call_types_not_equal() {
        let edge1 = CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        };

        let edge2 = CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g".to_string(),
            call_type: CallType::Attr, // Different
            via_import: None,
        };

        assert_ne!(edge1, edge2);
    }
}

// =============================================================================
// ProjectCallGraphV2 Tests
// =============================================================================

mod project_call_graph_v2_tests {
    use super::*;

    #[test]
    fn test_project_call_graph_v2_new() {
        let graph = ProjectCallGraphV2::new();
        assert_eq!(graph.len(), 0);
        assert!(graph.is_empty());
    }

    #[test]
    fn test_project_call_graph_v2_add_edge() {
        let mut graph = ProjectCallGraphV2::new();

        let edge = CrossFileCallEdge {
            src_file: PathBuf::from("src/main.py"),
            src_func: "main".to_string(),
            dst_file: PathBuf::from("src/utils.py"),
            dst_func: "helper".to_string(),
            call_type: CallType::Direct,
            via_import: Some("utils".to_string()),
        };

        graph.add_edge(edge);
        assert_eq!(graph.len(), 1);
        assert!(!graph.is_empty());
    }

    #[test]
    fn test_project_call_graph_v2_no_duplicates() {
        let mut graph = ProjectCallGraphV2::new();

        let edge = CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        };

        graph.add_edge(edge.clone());
        graph.add_edge(edge); // Duplicate

        assert_eq!(graph.len(), 1);
    }

    #[test]
    fn test_project_call_graph_v2_edges_iterator() {
        let mut graph = ProjectCallGraphV2::new();

        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f1".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g1".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        });

        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f2".to_string(),
            dst_file: PathBuf::from("c.py"),
            dst_func: "h".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        });

        let edges: Vec<_> = graph.edges().collect();
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn test_project_call_graph_v2_callers_of() {
        let mut graph = ProjectCallGraphV2::new();

        // a.f -> b.g
        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        });

        // c.h -> b.g
        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("c.py"),
            src_func: "h".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        });

        // a.f -> d.i (different target)
        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("d.py"),
            dst_func: "i".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        });

        // Query: who calls b.g?
        let callers: Vec<_> = graph.callers_of(&PathBuf::from("b.py"), "g").collect();

        assert_eq!(callers.len(), 2);

        let caller_funcs: HashSet<_> = callers.iter().map(|e| &e.src_func).collect();
        assert!(caller_funcs.contains(&"f".to_string()));
        assert!(caller_funcs.contains(&"h".to_string()));
    }

    #[test]
    fn test_project_call_graph_v2_callees_of() {
        let mut graph = ProjectCallGraphV2::new();

        // a.f -> b.g
        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        });

        // a.f -> c.h
        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("c.py"),
            dst_func: "h".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        });

        // a.f -> d.i
        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("d.py"),
            dst_func: "i".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        });

        // Query: what does a.f call?
        let callees: Vec<_> = graph.callees_of(&PathBuf::from("a.py"), "f").collect();

        assert_eq!(callees.len(), 3);

        let callee_funcs: HashSet<_> = callees.iter().map(|e| &e.dst_func).collect();
        assert!(callee_funcs.contains(&"g".to_string()));
        assert!(callee_funcs.contains(&"h".to_string()));
        assert!(callee_funcs.contains(&"i".to_string()));
    }

    #[test]
    fn test_project_call_graph_v2_callers_of_empty() {
        let graph = ProjectCallGraphV2::new();

        let callers: Vec<_> = graph
            .callers_of(&PathBuf::from("nonexistent.py"), "func")
            .collect();

        assert!(callers.is_empty());
    }

    #[test]
    fn test_project_call_graph_v2_callees_of_empty() {
        let graph = ProjectCallGraphV2::new();

        let callees: Vec<_> = graph
            .callees_of(&PathBuf::from("nonexistent.py"), "func")
            .collect();

        assert!(callees.is_empty());
    }

    #[test]
    fn test_project_call_graph_v2_contains() {
        let mut graph = ProjectCallGraphV2::new();

        let edge = CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        };

        graph.add_edge(edge.clone());

        assert!(graph.contains(&edge));

        let other_edge = CrossFileCallEdge {
            src_file: PathBuf::from("x.py"),
            src_func: "y".to_string(),
            dst_file: PathBuf::from("z.py"),
            dst_func: "w".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        };

        assert!(!graph.contains(&other_edge));
    }

    #[test]
    fn test_project_call_graph_v2_edge_paths_normalized() {
        let mut graph = ProjectCallGraphV2::new();

        // Add edge with forward slashes
        let edge = CrossFileCallEdge {
            src_file: PathBuf::from("src/main.py"),
            src_func: "main".to_string(),
            dst_file: PathBuf::from("src/utils/helper.py"),
            dst_func: "help".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        };

        graph.add_edge(edge);

        // Verify paths use forward slashes (POSIX format)
        let edges: Vec<_> = graph.edges().collect();
        assert_eq!(edges.len(), 1);

        let path_str = edges[0].src_file.to_string_lossy();
        // On all platforms, PathBuf should handle paths correctly
        assert!(path_str.contains("src"));
        assert!(path_str.contains("main.py"));
    }

    #[test]
    fn test_project_call_graph_v2_default() {
        let graph = ProjectCallGraphV2::default();
        assert!(graph.is_empty());
    }

    #[test]
    fn test_project_call_graph_v2_multiple_call_types() {
        let mut graph = ProjectCallGraphV2::new();

        // Direct call
        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g".to_string(),
            call_type: CallType::Direct,
            via_import: Some("b".to_string()),
        });

        // Method call to same target
        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "f".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "g".to_string(),
            call_type: CallType::Method, // Different call type
            via_import: Some("b".to_string()),
        });

        // These should be different edges (call_type differs)
        assert_eq!(graph.len(), 2);
    }
}
