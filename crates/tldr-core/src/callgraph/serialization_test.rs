//! Tests for Phase 15: Serialization
//!
//! These tests verify the JSON serialization implementation per
//! `migration/spec/phases-14-16-spec.md` Section 15.
//!
//! All tests are designed to fail initially (red phase of TDD) since
//! the implementation does not exist yet. They will pass once the
//! `serialization` module is implemented.

use std::path::PathBuf;

// Types from serialization module (to be created)
use super::cross_file_types::{
    CallGraphIR, CallSite, CallType, ClassDef, CrossFileCallEdge, FileIRBuilder, FuncDef,
    ImportDef, ProjectCallGraphV2, VarType,
};
use super::serialization::{SerializationError, IR_VERSION};

// =============================================================================
// Test Fixtures
// =============================================================================

/// Creates a minimal CallGraphIR for testing.
fn create_minimal_ir() -> CallGraphIR {
    CallGraphIR::new(PathBuf::from("/project"), "python")
}

/// Creates a CallGraphIR with typical content.
fn create_typical_ir() -> CallGraphIR {
    let mut ir = CallGraphIR::new(PathBuf::from("/project"), "python");

    // Add a file with functions
    let file_ir = FileIRBuilder::new(PathBuf::from("src/main.py"))
        .func(FuncDef::function("main", 1, 10))
        .func(FuncDef::method("save", "User", 15, 25))
        .class(ClassDef::simple("User", 12, 30))
        .import(ImportDef::from_import(
            "helper",
            vec!["process".to_string()],
        ))
        .call(CallSite::direct("main", "process", Some(5)))
        .call(CallSite::method("main", "save", "user", None, Some(7)))
        .build();

    ir.add_file(file_ir);
    ir.build_indices();
    ir
}

/// Creates a large CallGraphIR for stress testing.
fn create_large_ir(num_files: usize, funcs_per_file: usize) -> CallGraphIR {
    let mut ir = CallGraphIR::with_capacity(PathBuf::from("/large_project"), "python", num_files);

    for i in 0..num_files {
        let mut builder = FileIRBuilder::new(PathBuf::from(format!("src/module_{}.py", i)));

        for j in 0..funcs_per_file {
            builder = builder.func(FuncDef::function(
                format!("func_{}_{}", i, j),
                (j * 10 + 1) as u32,
                (j * 10 + 8) as u32,
            ));
        }

        ir.add_file(builder.build());
    }

    ir.build_indices();
    ir
}

// =============================================================================
// Phase 15.2: IR_VERSION Tests
// =============================================================================

mod ir_version {
    use super::*;

    /// Test: IR_VERSION is defined and is "1.0".
    /// Spec Section 15.2: 'pub const IR_VERSION: &str = "1.0"'
    #[test]
    fn test_ir_version_is_1_0() {
        assert_eq!(IR_VERSION, "1.0");
    }

    /// Test: Serialized JSON includes _version field.
    /// Spec Section 15.3: Output format includes '"_version": "1.0"'
    #[test]
    fn test_to_json_includes_version() {
        let ir = create_minimal_ir();
        let json = ir.to_json().unwrap();

        assert!(
            json.contains(r#""_version":"1.0""#) || json.contains(r#""_version": "1.0""#),
            "JSON should contain _version field"
        );
    }
}

// =============================================================================
// Phase 15.3: Serialization Methods Tests
// =============================================================================

mod serialization_methods {
    use super::*;

    /// Test: to_json produces valid JSON string.
    #[test]
    fn test_to_json_format() {
        let ir = create_typical_ir();
        let json_result = ir.to_json();

        assert!(json_result.is_ok(), "to_json should succeed");
        let json = json_result.unwrap();

        // Validate it's parseable JSON
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
        assert!(parsed.is_ok(), "Output should be valid JSON");

        let value = parsed.unwrap();
        assert!(value.is_object(), "Root should be an object");
        assert!(value["_version"].is_string(), "_version should be string");
        assert!(value["root"].is_string(), "root should be string");
        assert!(value["language"].is_string(), "language should be string");
        assert!(value["files"].is_object(), "files should be object");
    }

    /// Test: to_json_value returns serde_json::Value.
    #[test]
    fn test_to_json_value() {
        let ir = create_typical_ir();
        let value = ir.to_json_value();

        assert!(value.is_object());
        assert_eq!(value["_version"], "1.0");
        assert_eq!(value["language"], "python");
    }

    /// Test: from_json parses valid JSON.
    #[test]
    fn test_from_json_valid() {
        let json = r#"{
            "_version": "1.0",
            "root": "/project",
            "language": "python",
            "files": {}
        }"#;

        let result = CallGraphIR::from_json(json);
        assert!(
            result.is_ok(),
            "Valid JSON should parse: {:?}",
            result.err()
        );

        let ir = result.unwrap();
        assert_eq!(ir.language, "python");
        assert_eq!(ir.file_count(), 0);
    }

    /// Test: from_json rejects version mismatch.
    /// Spec Section 15.3: "IRVersionMismatch if _version doesn't match IR_VERSION"
    #[test]
    fn test_from_json_version_mismatch() {
        let json = r#"{
            "_version": "0.5",
            "root": "/project",
            "language": "python",
            "files": {}
        }"#;

        let result = CallGraphIR::from_json(json);
        assert!(result.is_err(), "Version mismatch should fail");

        match result.unwrap_err() {
            SerializationError::IRVersionMismatch { expected, actual } => {
                assert_eq!(expected, "1.0");
                assert_eq!(actual, "0.5");
            }
            other => panic!("Expected IRVersionMismatch, got {:?}", other),
        }
    }

    /// Test: from_json rejects missing _version.
    #[test]
    fn test_from_json_missing_version() {
        let json = r#"{
            "root": "/project",
            "language": "python",
            "files": {}
        }"#;

        let result = CallGraphIR::from_json(json);
        assert!(result.is_err(), "Missing _version should fail");

        assert!(matches!(
            result.unwrap_err(),
            SerializationError::MissingField(_)
        ));
    }

    /// Test: from_json rejects invalid JSON.
    #[test]
    fn test_from_json_invalid_format() {
        let json = "{ not valid json }";

        let result = CallGraphIR::from_json(json);
        assert!(result.is_err(), "Invalid JSON should fail");

        assert!(matches!(
            result.unwrap_err(),
            SerializationError::InvalidFormat(_) | SerializationError::Json(_)
        ));
    }

    /// Test: from_json_value works with serde_json::Value.
    #[test]
    fn test_from_json_value() {
        let value = serde_json::json!({
            "_version": "1.0",
            "root": "/project",
            "language": "python",
            "files": {}
        });

        let result = CallGraphIR::from_json_value(value);
        assert!(result.is_ok());
    }
}

// =============================================================================
// Phase 15.7: Round-Trip Tests
// =============================================================================

mod roundtrip {
    use super::*;

    /// Test: Round-trip identity for minimal IR.
    /// Spec Section 15.7: "from_json(to_json(ir)) == ir"
    #[test]
    fn test_roundtrip_identity() {
        let ir = create_typical_ir();

        let json = ir.to_json().unwrap();
        let ir2 = CallGraphIR::from_json(&json).unwrap();

        // Verify structural equality
        assert_eq!(ir.version, ir2.version);
        assert_eq!(ir.language, ir2.language);
        assert_eq!(ir.file_count(), ir2.file_count());
        assert_eq!(ir.function_count(), ir2.function_count());
        assert_eq!(ir.class_count(), ir2.class_count());

        // Verify file contents match
        for (path, file_ir) in &ir.files {
            let file_ir2 = ir2
                .files
                .get(path)
                .expect("File should exist after roundtrip");
            assert_eq!(file_ir.funcs.len(), file_ir2.funcs.len());
            assert_eq!(file_ir.classes.len(), file_ir2.classes.len());
            assert_eq!(file_ir.imports.len(), file_ir2.imports.len());
            assert_eq!(file_ir.calls.len(), file_ir2.calls.len());
        }
    }

    /// Test: Round-trip preserves function details.
    #[test]
    fn test_roundtrip_function_details() {
        let mut ir = CallGraphIR::new(PathBuf::from("/project"), "python");

        let file_ir = FileIRBuilder::new(PathBuf::from("module.py"))
            .func(FuncDef {
                name: "my_method".to_string(),
                line: 10,
                end_line: 20,
                is_method: true,
                class_name: Some("MyClass".to_string()),
                return_type: Some("str".to_string()),
                parent_function: Some("outer".to_string()),
            })
            .build();
        ir.add_file(file_ir);

        let json = ir.to_json().unwrap();
        let ir2 = CallGraphIR::from_json(&json).unwrap();

        let file = ir2.get_file("module.py").unwrap();
        let func = file.funcs.iter().find(|f| f.name == "my_method").unwrap();

        assert!(func.is_method);
        assert_eq!(func.class_name, Some("MyClass".to_string()));
        assert_eq!(func.return_type, Some("str".to_string()));
        assert_eq!(func.parent_function, Some("outer".to_string()));
    }

    /// Test: Round-trip preserves call site details.
    #[test]
    fn test_roundtrip_call_site_details() {
        let mut ir = CallGraphIR::new(PathBuf::from("/project"), "python");

        let file_ir = FileIRBuilder::new(PathBuf::from("module.py"))
            .call(CallSite {
                caller: "main".to_string(),
                target: "helper".to_string(),
                call_type: CallType::Method,
                line: Some(42),
                column: Some(8),
                receiver: Some("obj".to_string()),
                receiver_type: Some("MyClass".to_string()),
            })
            .build();
        ir.add_file(file_ir);

        let json = ir.to_json().unwrap();
        let ir2 = CallGraphIR::from_json(&json).unwrap();

        let file = ir2.get_file("module.py").unwrap();
        // Calls are stored in a HashMap by caller name
        let calls_from_main = file.calls.get("main").expect("Should have calls from main");
        let call = &calls_from_main[0];

        assert_eq!(call.call_type, CallType::Method);
        assert_eq!(call.line, Some(42));
        assert_eq!(call.receiver, Some("obj".to_string()));
        assert_eq!(call.receiver_type, Some("MyClass".to_string()));
    }
}

// =============================================================================
// Phase 15.5: Determinism Tests
// =============================================================================

mod determinism {
    use super::*;

    /// Test: to_json output is deterministic (sorted keys).
    /// Spec Section 15.3: "Deterministic output (sorted keys)"
    #[test]
    fn test_deterministic_output() {
        let ir = create_typical_ir();

        // Serialize multiple times
        let json1 = ir.to_json().unwrap();
        let json2 = ir.to_json().unwrap();
        let json3 = ir.to_json().unwrap();

        // All outputs should be identical
        assert_eq!(json1, json2, "Multiple serializations should be identical");
        assert_eq!(json2, json3, "Multiple serializations should be identical");
    }

    /// Test: Edge serialization is sorted.
    /// Spec Section 15.5: "Edges are sorted lexicographically for deterministic output"
    #[test]
    fn test_edges_sorted_for_determinism() {
        let mut graph = ProjectCallGraphV2::new();

        // Add edges in random order
        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("z.py"),
            src_func: "z_func".to_string(),
            dst_file: PathBuf::from("a.py"),
            dst_func: "a_func".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        });
        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "a_func".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "b_func".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        });

        let json = graph.edges_to_json();
        let edges = json.as_array().unwrap();

        // First edge should be a.py (sorted)
        let first_src = edges[0][0].as_str().unwrap();
        assert!(
            first_src.contains("a.py"),
            "First edge should be from a.py (sorted)"
        );
    }
}

// =============================================================================
// Phase 15: Large Graph Tests
// =============================================================================

mod large_graphs {
    use super::*;

    /// Test: Serialization handles large graphs.
    #[test]
    fn test_large_graph_serialization() {
        let ir = create_large_ir(100, 50); // 100 files, 50 funcs each = 5000 functions

        // Serialize
        let json_result = ir.to_json();
        assert!(
            json_result.is_ok(),
            "Large graph serialization should succeed"
        );

        let json = json_result.unwrap();

        // Deserialize
        let ir2_result = CallGraphIR::from_json(&json);
        assert!(
            ir2_result.is_ok(),
            "Large graph deserialization should succeed"
        );

        let ir2 = ir2_result.unwrap();
        assert_eq!(ir.file_count(), ir2.file_count());
        assert_eq!(ir.function_count(), ir2.function_count());
    }

    /// Test: Serialization performance is reasonable.
    #[test]
    fn test_serialization_performance() {
        let ir = create_large_ir(50, 20); // 1000 functions

        let start = std::time::Instant::now();
        let json = ir.to_json().unwrap();
        let serialize_time = start.elapsed();

        let start = std::time::Instant::now();
        let _ir2 = CallGraphIR::from_json(&json).unwrap();
        let deserialize_time = start.elapsed();

        // Reasonable performance expectations (adjust as needed)
        assert!(
            serialize_time.as_secs() < 5,
            "Serialization should complete in < 5s"
        );
        assert!(
            deserialize_time.as_secs() < 5,
            "Deserialization should complete in < 5s"
        );
    }
}

// =============================================================================
// Phase 15.4: JSON Schema Compliance Tests
// =============================================================================

mod json_schema {
    use super::*;

    /// Test: CallType serializes to lowercase.
    /// Spec Section 15.4: 'enum: ["intra", "direct", "method", "attr", "ref", "static"]'
    #[test]
    fn test_call_type_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&CallType::Intra).unwrap(),
            r#""intra""#
        );
        assert_eq!(
            serde_json::to_string(&CallType::Direct).unwrap(),
            r#""direct""#
        );
        assert_eq!(
            serde_json::to_string(&CallType::Method).unwrap(),
            r#""method""#
        );
        assert_eq!(serde_json::to_string(&CallType::Attr).unwrap(), r#""attr""#);
        assert_eq!(serde_json::to_string(&CallType::Ref).unwrap(), r#""ref""#);
        assert_eq!(
            serde_json::to_string(&CallType::Static).unwrap(),
            r#""static""#
        );
    }

    /// Test: All paths are normalized to POSIX format.
    /// Spec Section 15.3: "All paths as POSIX (forward slashes)"
    #[test]
    fn test_paths_normalized_to_posix() {
        let mut ir = CallGraphIR::new(PathBuf::from("C:\\project"), "python");

        let file_ir = FileIRBuilder::new(PathBuf::from("src\\module.py"))
            .func(FuncDef::function("foo", 1, 5))
            .build();
        ir.add_file(file_ir);

        let json = ir.to_json().unwrap();

        // Should not contain backslashes
        assert!(
            !json.contains("\\\\"),
            "JSON should not contain backslashes"
        );
        // Should contain forward slashes for paths
        assert!(
            json.contains("src/module.py") || json.contains("module.py"),
            "Paths should use forward slashes"
        );
    }

    /// Test: VarType serialization matches schema.
    #[test]
    fn test_var_type_serialization() {
        let var_type = VarType::new("user", "User", "annotation", 10);

        let json = serde_json::to_string(&var_type).unwrap();

        assert!(json.contains(r#""var_name":"user""#) || json.contains(r#""var_name": "user""#));
        assert!(json.contains(r#""type_name":"User""#) || json.contains(r#""type_name": "User""#));
        assert!(
            json.contains(r#""source":"annotation""#) || json.contains(r#""source": "annotation""#)
        );
        assert!(json.contains(r#""line":10"#) || json.contains(r#""line": 10"#));
    }
}

// =============================================================================
// Phase 15.6: Error Types Tests
// =============================================================================

mod error_types {
    use super::*;

    /// Test: SerializationError variants exist.
    #[test]
    fn test_error_variants_exist() {
        let _version_error = SerializationError::IRVersionMismatch {
            expected: "1.0".to_string(),
            actual: "0.5".to_string(),
        };

        let _format_error = SerializationError::InvalidFormat("test".to_string());
        let _field_error = SerializationError::MissingField("_version".to_string());
    }

    /// Test: Error messages are descriptive.
    #[test]
    fn test_error_messages() {
        let error = SerializationError::IRVersionMismatch {
            expected: "1.0".to_string(),
            actual: "0.5".to_string(),
        };

        let message = error.to_string();
        assert!(
            message.contains("1.0"),
            "Error should mention expected version"
        );
        assert!(
            message.contains("0.5"),
            "Error should mention actual version"
        );
    }
}
