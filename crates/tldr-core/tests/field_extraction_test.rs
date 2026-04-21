//! Tests for Gap 3: FieldInfo type and field/constant extraction.
//!
//! These tests define the expected behavior for the FieldInfo type,
//! ClassInfo.fields, and ModuleInfo.constants. They are written BEFORE
//! the implementation exists, following TDD methodology.
//!
//! EXPECTED: These tests will FAIL AT COMPILE TIME until:
//!   1. FieldInfo struct is added to types.rs
//!   2. ClassInfo gains a `fields: Vec<FieldInfo>` field
//!   3. ModuleInfo gains a `constants: Vec<FieldInfo>` field
//!   4. Extraction logic is implemented in extract.rs
//!
//! See: target/gap3-fieldinfo-spec.md for the full behavioral specification.

use tldr_core::ast::extract::extract_file;
use tldr_core::types::{ClassInfo, FieldInfo, ModuleInfo};

/// Helper: create a temp file with given content and extension, then extract it.
fn extract_source(source: &str, extension: &str) -> tldr_core::TldrResult<ModuleInfo> {
    let dir = tempfile::TempDir::new().unwrap();
    let filename = format!("test_input{}", extension);
    let file = dir.path().join(&filename);
    std::fs::write(&file, source).unwrap();
    extract_file(&file, None)
}

// =============================================================================
// 1. Python class fields
// =============================================================================

mod python_class_fields {
    use super::*;

    #[test]
    fn extracts_class_level_assignment_as_static_field() {
        // GIVEN: A Python class with a class-level assignment
        let source = r#"
class Foo:
    x = 10
    y: str = "hello"
"#;

        // WHEN: We extract the module
        let info = extract_source(source, ".py").unwrap();

        // THEN: ClassInfo should have fields for x and y
        assert_eq!(info.classes.len(), 1);
        let foo = &info.classes[0];
        assert_eq!(foo.name, "Foo");
        assert!(
            foo.fields.len() >= 2,
            "Expected at least 2 fields, got {}",
            foo.fields.len()
        );

        let field_x = foo
            .fields
            .iter()
            .find(|f| f.name == "x")
            .expect("Field 'x' not found");
        assert_eq!(field_x.default_value.as_deref(), Some("10"));
        assert!(field_x.is_static, "Class-level assignment should be static");
        assert!(!field_x.is_constant, "Lowercase 'x' should not be constant");

        let field_y = foo
            .fields
            .iter()
            .find(|f| f.name == "y")
            .expect("Field 'y' not found");
        assert_eq!(field_y.field_type.as_deref(), Some("str"));
        assert_eq!(field_y.default_value.as_deref(), Some("\"hello\""));
        assert!(field_y.is_static, "Class-level assignment should be static");
    }

    #[test]
    fn detects_upper_case_class_variable_as_constant() {
        // GIVEN: A Python class with an UPPER_CASE class variable
        let source = r#"
class Config:
    MAX_SIZE = 100
    api_url = "http://example.com"
"#;

        // WHEN: We extract the module
        let info = extract_source(source, ".py").unwrap();
        let config = &info.classes[0];

        // THEN: MAX_SIZE should be marked as constant, api_url should not
        let max_size = config
            .fields
            .iter()
            .find(|f| f.name == "MAX_SIZE")
            .expect("Field 'MAX_SIZE' not found");
        assert!(
            max_size.is_constant,
            "UPPER_CASE should be marked as constant"
        );
        assert!(max_size.is_static);

        let api_url = config
            .fields
            .iter()
            .find(|f| f.name == "api_url")
            .expect("Field 'api_url' not found");
        assert!(
            !api_url.is_constant,
            "lowercase should NOT be marked as constant"
        );
    }
}

// =============================================================================
// 2. Python instance fields (self.x in __init__)
// =============================================================================

mod python_instance_fields {
    use super::*;

    #[test]
    fn extracts_self_assignments_from_init() {
        // GIVEN: A Python class with self.x assignments in __init__
        let source = r#"
class Bar:
    def __init__(self):
        self.count = 0
        self.name = "default"
"#;

        // WHEN: We extract the module
        let info = extract_source(source, ".py").unwrap();

        // THEN: ClassInfo should have instance fields for count and name
        assert_eq!(info.classes.len(), 1);
        let bar = &info.classes[0];

        let count = bar
            .fields
            .iter()
            .find(|f| f.name == "count")
            .expect("Field 'count' not found");
        assert!(!count.is_static, "Instance field should NOT be static");
        assert_eq!(count.default_value.as_deref(), Some("0"));

        let name = bar
            .fields
            .iter()
            .find(|f| f.name == "name")
            .expect("Field 'name' not found");
        assert!(!name.is_static, "Instance field should NOT be static");
    }

    #[test]
    fn mixed_class_and_instance_fields() {
        // GIVEN: A class with both class-level and instance-level fields
        let source = r#"
class Mixed:
    MAX_RETRIES = 3
    x: int = 5

    def __init__(self):
        self.y = 10
"#;

        // WHEN: We extract
        let info = extract_source(source, ".py").unwrap();
        let mixed = &info.classes[0];

        // THEN: Should have 3 fields total
        assert!(
            mixed.fields.len() >= 3,
            "Expected at least 3 fields (MAX_RETRIES, x, y), got {}",
            mixed.fields.len()
        );

        // MAX_RETRIES: static + constant
        let max_r = mixed
            .fields
            .iter()
            .find(|f| f.name == "MAX_RETRIES")
            .unwrap();
        assert!(max_r.is_static);
        assert!(max_r.is_constant);

        // x: static, not constant
        let x = mixed.fields.iter().find(|f| f.name == "x").unwrap();
        assert!(x.is_static);
        assert!(!x.is_constant);

        // y: instance field
        let y = mixed.fields.iter().find(|f| f.name == "y").unwrap();
        assert!(!y.is_static);
    }

    #[test]
    fn private_field_by_underscore_convention() {
        // GIVEN: A class with a leading-underscore field
        let source = r#"
class Private:
    _secret: str = "hidden"
    public_val: int = 42
"#;

        // WHEN: We extract
        let info = extract_source(source, ".py").unwrap();
        let cls = &info.classes[0];

        // THEN: _secret should have visibility "private"
        let secret = cls.fields.iter().find(|f| f.name == "_secret").unwrap();
        assert_eq!(secret.visibility.as_deref(), Some("private"));

        let public = cls.fields.iter().find(|f| f.name == "public_val").unwrap();
        assert_eq!(public.visibility.as_deref(), Some("public"));
    }
}

// =============================================================================
// 3. Rust struct fields
// =============================================================================

mod rust_struct_fields {
    use super::*;

    #[test]
    fn extracts_struct_fields_with_visibility() {
        // GIVEN: A Rust struct with pub and private fields
        let source = r#"
struct Point {
    pub x: f64,
    y: f64,
}
"#;

        // WHEN: We extract
        let info = extract_source(source, ".rs").unwrap();

        // THEN: Should have a class (struct) with 2 fields
        assert!(
            !info.classes.is_empty(),
            "Should extract Rust struct as ClassInfo"
        );
        let point = info
            .classes
            .iter()
            .find(|c| c.name == "Point")
            .expect("Struct 'Point' not found");

        assert_eq!(
            point.fields.len(),
            2,
            "Expected 2 fields, got {}",
            point.fields.len()
        );

        let field_x = point
            .fields
            .iter()
            .find(|f| f.name == "x")
            .expect("Field 'x' not found");
        assert_eq!(field_x.field_type.as_deref(), Some("f64"));
        assert_eq!(field_x.visibility.as_deref(), Some("public"));

        let field_y = point
            .fields
            .iter()
            .find(|f| f.name == "y")
            .expect("Field 'y' not found");
        assert_eq!(field_y.field_type.as_deref(), Some("f64"));
        assert_eq!(field_y.visibility.as_deref(), Some("private"));
    }

    #[test]
    fn extracts_pub_crate_visibility() {
        // GIVEN: A Rust struct with pub(crate) field
        let source = r#"
struct Config {
    pub(crate) host: String,
    port: u16,
}
"#;

        // WHEN: We extract
        let info = extract_source(source, ".rs").unwrap();
        let config = info.classes.iter().find(|c| c.name == "Config").unwrap();

        // THEN: host should have pub(crate) visibility
        let host = config.fields.iter().find(|f| f.name == "host").unwrap();
        assert_eq!(host.field_type.as_deref(), Some("String"));
        assert_eq!(host.visibility.as_deref(), Some("pub(crate)"));

        let port = config.fields.iter().find(|f| f.name == "port").unwrap();
        assert_eq!(port.visibility.as_deref(), Some("private"));
    }

    #[test]
    fn struct_fields_have_line_numbers() {
        // GIVEN: A Rust struct
        let source = "struct Pair {\n    a: i32,\n    b: i32,\n}\n";

        // WHEN: We extract
        let info = extract_source(source, ".rs").unwrap();
        let pair = info.classes.iter().find(|c| c.name == "Pair").unwrap();

        // THEN: Fields should have line numbers (1-indexed)
        for field in &pair.fields {
            assert!(
                field.line_number > 0,
                "Field '{}' should have a nonzero line number",
                field.name
            );
        }
    }

    #[test]
    fn struct_fields_not_static_or_constant() {
        // GIVEN: Regular struct fields (not const/static)
        let source = "struct S { pub val: u32 }\n";

        // WHEN: We extract
        let info = extract_source(source, ".rs").unwrap();
        let s = info.classes.iter().find(|c| c.name == "S").unwrap();

        // THEN: Struct fields should not be marked static or constant
        let val = s.fields.iter().find(|f| f.name == "val").unwrap();
        assert!(!val.is_static);
        assert!(!val.is_constant);
    }

    #[test]
    fn generic_struct_field_types() {
        // GIVEN: A struct with generic field types
        let source = r#"
pub struct Cache<T> {
    pub data: Vec<T>,
    size: usize,
}
"#;

        // WHEN: We extract
        let info = extract_source(source, ".rs").unwrap();
        let cache = info.classes.iter().find(|c| c.name == "Cache").unwrap();

        // THEN: Field type should include generics
        let data = cache.fields.iter().find(|f| f.name == "data").unwrap();
        assert_eq!(data.field_type.as_deref(), Some("Vec<T>"));
    }
}

// =============================================================================
// 4. Go struct fields
// =============================================================================

mod go_struct_fields {
    use super::*;

    #[test]
    fn extracts_go_struct_fields_with_visibility() {
        // GIVEN: A Go struct with exported and unexported fields
        let source = r#"package main

type Config struct {
    Host string
    Port int
}
"#;

        // WHEN: We extract
        let info = extract_source(source, ".go").unwrap();

        // THEN: Should have struct fields with visibility based on case
        let config = info
            .classes
            .iter()
            .find(|c| c.name == "Config")
            .expect("Struct 'Config' not found in classes");

        assert_eq!(
            config.fields.len(),
            2,
            "Expected 2 fields, got {}",
            config.fields.len()
        );

        let host = config
            .fields
            .iter()
            .find(|f| f.name == "Host")
            .expect("Field 'Host' not found");
        assert_eq!(host.field_type.as_deref(), Some("string"));
        assert_eq!(host.visibility.as_deref(), Some("public"));

        let port = config
            .fields
            .iter()
            .find(|f| f.name == "Port")
            .expect("Field 'Port' not found");
        assert_eq!(port.field_type.as_deref(), Some("int"));
        assert_eq!(port.visibility.as_deref(), Some("public"));
    }

    #[test]
    fn unexported_go_fields_are_private() {
        // GIVEN: A Go struct with lowercase (unexported) fields
        let source = r#"package main

type internal struct {
    count int
    name  string
}
"#;

        // WHEN: We extract
        let info = extract_source(source, ".go").unwrap();
        let internal = info
            .classes
            .iter()
            .find(|c| c.name == "internal")
            .expect("Struct 'internal' not found");

        // THEN: Lowercase fields should be private
        let count = internal.fields.iter().find(|f| f.name == "count").unwrap();
        assert_eq!(count.visibility.as_deref(), Some("private"));

        let name = internal.fields.iter().find(|f| f.name == "name").unwrap();
        assert_eq!(name.visibility.as_deref(), Some("private"));
    }

    #[test]
    fn handles_multiple_fields_per_line() {
        // GIVEN: A Go struct with multiple field names on one line
        let source = r#"package main

type Point struct {
    X, Y int
}
"#;

        // WHEN: We extract
        let info = extract_source(source, ".go").unwrap();
        let point = info
            .classes
            .iter()
            .find(|c| c.name == "Point")
            .expect("Struct 'Point' not found");

        // THEN: Should produce separate FieldInfo for X and Y
        assert_eq!(
            point.fields.len(),
            2,
            "Expected 2 fields (X, Y), got {}",
            point.fields.len()
        );

        let x = point
            .fields
            .iter()
            .find(|f| f.name == "X")
            .expect("Field 'X' not found");
        assert_eq!(x.field_type.as_deref(), Some("int"));
        assert_eq!(x.visibility.as_deref(), Some("public"));

        let y = point
            .fields
            .iter()
            .find(|f| f.name == "Y")
            .expect("Field 'Y' not found");
        assert_eq!(y.field_type.as_deref(), Some("int"));
        assert_eq!(y.visibility.as_deref(), Some("public"));
    }
}

// =============================================================================
// 5. TypeScript class fields
// =============================================================================

mod typescript_class_fields {
    use super::*;

    #[test]
    fn extracts_ts_class_fields_with_modifiers() {
        // GIVEN: A TypeScript class with private and static fields
        let source = r#"
class App {
    private name: string = "test";
    static count: number = 0;
}
"#;

        // WHEN: We extract
        let info = extract_source(source, ".ts").unwrap();
        let app = info
            .classes
            .iter()
            .find(|c| c.name == "App")
            .expect("Class 'App' not found");

        // THEN: Should have 2 fields with correct modifiers
        assert!(
            app.fields.len() >= 2,
            "Expected at least 2 fields, got {}",
            app.fields.len()
        );

        let name = app
            .fields
            .iter()
            .find(|f| f.name == "name")
            .expect("Field 'name' not found");
        assert_eq!(name.field_type.as_deref(), Some("string"));
        assert_eq!(name.visibility.as_deref(), Some("private"));
        assert_eq!(name.default_value.as_deref(), Some("\"test\""));

        let count = app
            .fields
            .iter()
            .find(|f| f.name == "count")
            .expect("Field 'count' not found");
        assert_eq!(count.field_type.as_deref(), Some("number"));
        assert!(count.is_static, "static field should be marked is_static");
    }

    #[test]
    fn static_upper_case_is_constant() {
        // GIVEN: A TypeScript class with a static UPPER_CASE field
        let source = r#"
class Constants {
    static MAX_SIZE: number = 1024;
    static lowercaseStatic: number = 0;
}
"#;

        // WHEN: We extract
        let info = extract_source(source, ".ts").unwrap();
        let cls = info.classes.iter().find(|c| c.name == "Constants").unwrap();

        // THEN: MAX_SIZE should be constant, lowercaseStatic should not
        let max = cls.fields.iter().find(|f| f.name == "MAX_SIZE").unwrap();
        assert!(max.is_static);
        assert!(max.is_constant, "static UPPER_CASE should be constant");

        let lower = cls
            .fields
            .iter()
            .find(|f| f.name == "lowercaseStatic")
            .unwrap();
        assert!(lower.is_static);
        assert!(
            !lower.is_constant,
            "lowercase static should NOT be constant"
        );
    }

    #[test]
    fn default_visibility_is_public() {
        // GIVEN: A TypeScript class with no explicit modifier
        let source = r#"
class Simple {
    value: number = 42;
}
"#;

        // WHEN: We extract
        let info = extract_source(source, ".ts").unwrap();
        let cls = info.classes.iter().find(|c| c.name == "Simple").unwrap();

        // THEN: Default visibility should be public
        let val = cls.fields.iter().find(|f| f.name == "value").unwrap();
        assert_eq!(val.visibility.as_deref(), Some("public"));
    }
}

// =============================================================================
// 6. Java class fields
// =============================================================================

mod java_class_fields {
    use super::*;

    #[test]
    fn extracts_java_fields_with_visibility() {
        // GIVEN: A Java class with private and public fields
        let source = r#"
class User {
    private String name;
    public int age = 0;
}
"#;

        // WHEN: We extract
        let info = extract_source(source, ".java").unwrap();
        let user = info
            .classes
            .iter()
            .find(|c| c.name == "User")
            .expect("Class 'User' not found");

        // THEN: Should have 2 fields with correct visibility
        assert!(
            user.fields.len() >= 2,
            "Expected at least 2 fields, got {}",
            user.fields.len()
        );

        let name = user
            .fields
            .iter()
            .find(|f| f.name == "name")
            .expect("Field 'name' not found");
        assert_eq!(name.field_type.as_deref(), Some("String"));
        assert_eq!(name.visibility.as_deref(), Some("private"));
        assert!(name.default_value.is_none(), "name has no default value");

        let age = user
            .fields
            .iter()
            .find(|f| f.name == "age")
            .expect("Field 'age' not found");
        assert_eq!(age.field_type.as_deref(), Some("int"));
        assert_eq!(age.visibility.as_deref(), Some("public"));
        assert_eq!(age.default_value.as_deref(), Some("0"));
    }

    #[test]
    fn static_final_is_constant() {
        // GIVEN: A Java class with a static final field
        let source = r#"
class Settings {
    static final int MAX_SIZE = 100;
    private static int counter = 0;
}
"#;

        // WHEN: We extract
        let info = extract_source(source, ".java").unwrap();
        let cls = info.classes.iter().find(|c| c.name == "Settings").unwrap();

        // THEN: static final should be constant
        let max = cls.fields.iter().find(|f| f.name == "MAX_SIZE").unwrap();
        assert!(max.is_static);
        assert!(max.is_constant, "static final should be constant");

        let counter = cls.fields.iter().find(|f| f.name == "counter").unwrap();
        assert!(counter.is_static);
        assert!(
            !counter.is_constant,
            "static without final should NOT be constant"
        );
    }

    #[test]
    fn no_modifier_defaults_to_package_visibility() {
        // GIVEN: A Java class field with no modifier
        let source = r#"
class Defaults {
    int value = 42;
}
"#;

        // WHEN: We extract
        let info = extract_source(source, ".java").unwrap();
        let cls = info.classes.iter().find(|c| c.name == "Defaults").unwrap();

        // THEN: No modifier means package-private
        let val = cls.fields.iter().find(|f| f.name == "value").unwrap();
        assert_eq!(val.visibility.as_deref(), Some("package"));
    }
}

// =============================================================================
// 7. Module constants - Python
// =============================================================================

mod python_module_constants {
    use super::*;

    #[test]
    fn extracts_upper_case_module_level_assignments() {
        // GIVEN: A Python file with top-level UPPER_CASE constants
        let source = r#"
MAX_SIZE = 100
API_KEY = "secret"
config = load_config()
"#;

        // WHEN: We extract
        let info = extract_source(source, ".py").unwrap();

        // THEN: constants should contain MAX_SIZE and API_KEY but not config
        assert!(
            info.constants.len() >= 2,
            "Expected at least 2 constants, got {}",
            info.constants.len()
        );

        let max_size = info
            .constants
            .iter()
            .find(|c| c.name == "MAX_SIZE")
            .expect("Constant 'MAX_SIZE' not found");
        assert!(max_size.is_constant);
        assert!(max_size.is_static);
        assert_eq!(max_size.default_value.as_deref(), Some("100"));

        let api_key = info
            .constants
            .iter()
            .find(|c| c.name == "API_KEY")
            .expect("Constant 'API_KEY' not found");
        assert!(api_key.is_constant);

        // config should NOT appear (lowercase = not a constant)
        let config = info.constants.iter().find(|c| c.name == "config");
        assert!(
            config.is_none(),
            "Lowercase 'config' should not be in constants"
        );
    }

    #[test]
    fn constants_have_line_numbers() {
        let source = "MAX = 1\nMIN = 0\n";
        let info = extract_source(source, ".py").unwrap();

        for c in &info.constants {
            assert!(
                c.line_number > 0,
                "Constant '{}' should have a line number",
                c.name
            );
        }
    }
}

// =============================================================================
// 8. Module constants - Rust
// =============================================================================

mod rust_module_constants {
    use super::*;

    #[test]
    fn extracts_const_items() {
        // GIVEN: A Rust file with const declarations
        let source = r#"
pub const MAX: u32 = 100;
const MIN: u32 = 0;
"#;

        // WHEN: We extract
        let info = extract_source(source, ".rs").unwrap();

        // THEN: constants should contain MAX and MIN
        assert!(
            info.constants.len() >= 2,
            "Expected at least 2 constants, got {}",
            info.constants.len()
        );

        let max = info
            .constants
            .iter()
            .find(|c| c.name == "MAX")
            .expect("Constant 'MAX' not found");
        assert!(max.is_constant);
        assert_eq!(max.field_type.as_deref(), Some("u32"));
        assert_eq!(max.default_value.as_deref(), Some("100"));
        assert_eq!(max.visibility.as_deref(), Some("public"));

        let min = info
            .constants
            .iter()
            .find(|c| c.name == "MIN")
            .expect("Constant 'MIN' not found");
        assert!(min.is_constant);
        assert_eq!(min.visibility.as_deref(), Some("private"));
    }

    #[test]
    fn extracts_static_items() {
        // GIVEN: A Rust file with static declarations
        let source = "static GLOBAL: i32 = 42;\n";

        // WHEN: We extract
        let info = extract_source(source, ".rs").unwrap();

        // THEN: GLOBAL should appear in constants
        let global = info
            .constants
            .iter()
            .find(|c| c.name == "GLOBAL")
            .expect("Constant 'GLOBAL' not found");
        assert!(global.is_constant);
        assert!(global.is_static);
        assert_eq!(global.field_type.as_deref(), Some("i32"));
    }
}

// =============================================================================
// 9. Serde backward compatibility
// =============================================================================

mod serde_backward_compat {
    use super::*;

    #[test]
    fn old_json_without_fields_key_deserializes_to_empty_vec() {
        // GIVEN: JSON from the old format (no "fields" key in ClassInfo)
        let old_json = r#"{
            "name": "Foo",
            "bases": [],
            "methods": [],
            "decorators": [],
            "line_number": 10
        }"#;

        // WHEN: We deserialize
        let class: ClassInfo = serde_json::from_str(old_json).unwrap();

        // THEN: fields should default to empty
        assert!(
            class.fields.is_empty(),
            "Old JSON without 'fields' should deserialize to empty Vec"
        );
    }

    #[test]
    fn old_json_without_constants_key_deserializes_to_empty_vec() {
        // GIVEN: JSON from the old format (no "constants" key in ModuleInfo)
        let old_json = r#"{
            "file_path": "test.py",
            "language": "python",
            "imports": [],
            "functions": [],
            "classes": [],
            "call_graph": {"calls": {}, "called_by": {}}
        }"#;

        // WHEN: We deserialize
        let module: ModuleInfo = serde_json::from_str(old_json).unwrap();

        // THEN: constants should default to empty
        assert!(
            module.constants.is_empty(),
            "Old JSON without 'constants' should deserialize to empty Vec"
        );
    }

    #[test]
    fn empty_fields_omitted_from_json() {
        // GIVEN: A ClassInfo with empty fields
        let class = ClassInfo {
            name: "Foo".to_string(),
            bases: vec![],
            docstring: None,
            methods: vec![],
            fields: vec![],
            decorators: vec![],
            line_number: 10,
        };

        // WHEN: We serialize
        let json = serde_json::to_string(&class).unwrap();

        // THEN: "fields" key should NOT appear in output (skip_serializing_if = Vec::is_empty)
        assert!(
            !json.contains("\"fields\""),
            "Empty fields should be omitted from JSON, got: {}",
            json
        );
    }

    #[test]
    fn non_empty_fields_present_in_json() {
        // GIVEN: A ClassInfo with one field
        let class = ClassInfo {
            name: "Bar".to_string(),
            bases: vec![],
            docstring: None,
            methods: vec![],
            fields: vec![FieldInfo {
                name: "x".to_string(),
                field_type: Some("int".to_string()),
                default_value: Some("0".to_string()),
                is_static: false,
                is_constant: false,
                visibility: Some("public".to_string()),
                line_number: 2,
            }],
            decorators: vec![],
            line_number: 1,
        };

        // WHEN: We serialize and deserialize
        let json = serde_json::to_string(&class).unwrap();
        let back: ClassInfo = serde_json::from_str(&json).unwrap();

        // THEN: Fields should round-trip correctly
        assert!(
            json.contains("\"fields\""),
            "Non-empty fields should be in JSON"
        );
        assert_eq!(back.fields.len(), 1);
        assert_eq!(back.fields[0].name, "x");
        assert_eq!(back.fields[0].field_type.as_deref(), Some("int"));
        assert_eq!(back.fields[0].default_value.as_deref(), Some("0"));
        assert_eq!(back.fields[0].visibility.as_deref(), Some("public"));
        assert_eq!(back.fields[0].line_number, 2);
    }

    #[test]
    fn fieldinfo_skips_none_fields_in_json() {
        // GIVEN: A FieldInfo with None optional fields
        let field = FieldInfo {
            name: "count".to_string(),
            field_type: None,
            default_value: None,
            is_static: false,
            is_constant: false,
            visibility: None,
            line_number: 5,
        };

        // WHEN: We serialize
        let json = serde_json::to_string(&field).unwrap();

        // THEN: None fields should be omitted
        assert!(
            !json.contains("field_type"),
            "None field_type should be omitted"
        );
        assert!(
            !json.contains("default_value"),
            "None default_value should be omitted"
        );
        assert!(
            !json.contains("visibility"),
            "None visibility should be omitted"
        );

        // But name and line_number should always be present
        assert!(json.contains("\"name\""));
        assert!(json.contains("\"line_number\""));
    }

    #[test]
    fn fieldinfo_serde_roundtrip() {
        // GIVEN: A fully populated FieldInfo
        let field = FieldInfo {
            name: "MAX_SIZE".to_string(),
            field_type: Some("u32".to_string()),
            default_value: Some("100".to_string()),
            is_static: true,
            is_constant: true,
            visibility: Some("public".to_string()),
            line_number: 3,
        };

        // WHEN: We serialize and deserialize
        let json = serde_json::to_string(&field).unwrap();
        let back: FieldInfo = serde_json::from_str(&json).unwrap();

        // THEN: All fields should round-trip
        assert_eq!(back.name, "MAX_SIZE");
        assert_eq!(back.field_type.as_deref(), Some("u32"));
        assert_eq!(back.default_value.as_deref(), Some("100"));
        assert!(back.is_static);
        assert!(back.is_constant);
        assert_eq!(back.visibility.as_deref(), Some("public"));
        assert_eq!(back.line_number, 3);
    }

    #[test]
    fn fieldinfo_defaults_for_bool_fields() {
        // GIVEN: JSON with is_static and is_constant omitted (should default to false)
        let json = r#"{"name":"x","line_number":1}"#;

        // WHEN: We deserialize
        let field: FieldInfo = serde_json::from_str(json).unwrap();

        // THEN: Booleans should default to false
        assert!(!field.is_static, "is_static should default to false");
        assert!(!field.is_constant, "is_constant should default to false");
    }
}
