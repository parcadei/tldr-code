use tldr_core::types::{DefinitionInfo, FileStructure};
use tldr_core::{get_code_structure, Language};

#[test]
fn test_definition_info_serde_roundtrip() {
    let def = DefinitionInfo {
        name: "foo".to_string(),
        kind: "function".to_string(),
        line_start: 1,
        line_end: 10,
        signature: "pub fn foo(x: i32) -> bool".to_string(),
    };
    let json = serde_json::to_string(&def).unwrap();
    let back: DefinitionInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(def, back);
}

#[test]
fn test_file_structure_definitions_default_empty() {
    // Old JSON format without "definitions" field should deserialize with empty vec
    let json = r#"{"path":"test.py","functions":["foo"],"classes":[],"methods":[],"imports":[]}"#;
    let fs: FileStructure = serde_json::from_str(json).unwrap();
    assert!(fs.definitions.is_empty());
}

#[test]
fn test_file_structure_definitions_skip_when_empty() {
    // When definitions is empty, it should NOT appear in serialized JSON
    let fs = FileStructure {
        path: std::path::PathBuf::from("test.py"),
        functions: vec!["foo".to_string()],
        classes: vec![],
        methods: vec![],
        imports: vec![],
        definitions: vec![],
    };
    let json = serde_json::to_string(&fs).unwrap();
    assert!(
        !json.contains("definitions"),
        "Empty definitions should be skipped in JSON"
    );
}

#[test]
fn test_extract_populates_definitions_python() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("test.py");
    std::fs::write(&file, "def foo(x, y):\n    return x + y\n\nclass Bar:\n    def method(self):\n        pass\n\ndef baz():\n    pass\n").unwrap();

    let structure = get_code_structure(dir.path(), Language::Python, 0, None).unwrap();
    assert_eq!(structure.files.len(), 1);
    let defs = &structure.files[0].definitions;
    assert!(
        defs.len() >= 3,
        "Expected at least 3 definitions (foo, Bar, baz), got {}",
        defs.len()
    );

    for def in defs {
        assert!(!def.name.is_empty(), "Definition name should not be empty");
        assert!(!def.kind.is_empty(), "Definition kind should not be empty");
        assert!(def.line_start > 0, "line_start should be 1-indexed");
        assert!(
            def.line_end >= def.line_start,
            "line_end should be >= line_start"
        );
        assert!(!def.signature.is_empty(), "Signature should not be empty");
    }
}

#[test]
fn test_extract_populates_definitions_rust() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("test.rs");
    std::fs::write(&file, "pub fn hello(name: &str) -> String {\n    format!(\"Hello, {}\", name)\n}\n\nstruct Point {\n    x: f64,\n    y: f64,\n}\n\nfn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n").unwrap();

    let structure = get_code_structure(dir.path(), Language::Rust, 0, None).unwrap();
    assert_eq!(structure.files.len(), 1);
    let defs = &structure.files[0].definitions;
    assert!(
        defs.len() >= 2,
        "Expected at least 2 definitions (hello, add or Point), got {}",
        defs.len()
    );

    // Check that function definitions have correct kind
    let funcs: Vec<_> = defs.iter().filter(|d| d.kind == "function").collect();
    assert!(
        !funcs.is_empty(),
        "Should have at least one function definition"
    );
}

#[test]
fn test_definitions_line_ranges_correct() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("lines.py");
    // Line 1: def foo
    // Line 2:     return 1
    // Line 3: (blank)
    // Line 4: def bar
    // Line 5:     return 2
    std::fs::write(
        &file,
        "def foo():\n    return 1\n\ndef bar():\n    return 2\n",
    )
    .unwrap();

    let structure = get_code_structure(dir.path(), Language::Python, 0, None).unwrap();
    let defs = &structure.files[0].definitions;

    let foo = defs
        .iter()
        .find(|d| d.name == "foo")
        .expect("Should find foo");
    assert_eq!(foo.line_start, 1);
    assert!(
        foo.line_end >= 1 && foo.line_end <= 2,
        "foo should end at line 1 or 2, got {}",
        foo.line_end
    );

    let bar = defs
        .iter()
        .find(|d| d.name == "bar")
        .expect("Should find bar");
    assert_eq!(bar.line_start, 4);
    assert!(
        bar.line_end >= 4 && bar.line_end <= 5,
        "bar should end at line 4 or 5, got {}",
        bar.line_end
    );
}

#[test]
fn test_definitions_names_match_functions_field() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("match.py");
    std::fs::write(&file, "def alpha():\n    pass\n\ndef beta():\n    pass\n").unwrap();

    let structure = get_code_structure(dir.path(), Language::Python, 0, None).unwrap();
    let fs = &structure.files[0];

    // Every function name in the `functions` field should appear in `definitions`
    for func_name in &fs.functions {
        let found = fs
            .definitions
            .iter()
            .any(|d| &d.name == func_name && d.kind == "function");
        assert!(
            found,
            "Function '{}' from functions field not found in definitions",
            func_name
        );
    }
}
