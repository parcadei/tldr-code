//! Tests for Gap 1: Rust impl->struct method association
//!
//! These tests verify that `extract_from_tree` correctly associates methods
//! defined in `impl` blocks with their corresponding struct/enum ClassInfo.
//!
//! Current behavior (BROKEN): ClassInfo.methods is always empty for Rust.
//! Expected behavior: Methods from impl blocks populate ClassInfo.methods.
//!
//! All tests in this file are expected to FAIL until the fix is implemented.

use std::path::Path;

use tldr_core::ast::extract::extract_from_tree;
use tldr_core::ast::parser::parse;
use tldr_core::Language;

/// Helper: parse Rust source and extract ModuleInfo
fn extract_rust(source: &str) -> tldr_core::ModuleInfo {
    let tree = parse(source, Language::Rust).expect("Failed to parse Rust source");
    let dummy_path = Path::new("test.rs");
    extract_from_tree(&tree, source, Language::Rust, dummy_path, None)
        .expect("Failed to extract from tree")
}

// =============================================================================
// Contract 1: Basic impl->struct association
// =============================================================================

mod basic_impl_association {
    use super::*;

    #[test]
    fn struct_with_impl_has_methods_populated() {
        // GIVEN: A struct with an impl block containing two methods
        let source = r#"
struct Animal {
    name: String,
}

impl Animal {
    fn new(name: &str) -> Self {
        Animal { name: name.to_string() }
    }

    fn speak(&self) -> &str {
        &self.name
    }
}
"#;

        // WHEN: We extract module info
        let info = extract_rust(source);

        // THEN: ClassInfo for Animal should have 2 methods
        assert_eq!(info.classes.len(), 1, "Expected 1 class (Animal)");
        let animal = &info.classes[0];
        assert_eq!(animal.name, "Animal");
        assert_eq!(
            animal.methods.len(),
            2,
            "Animal should have 2 methods (new, speak), got: {:?}",
            animal.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn impl_methods_have_correct_names() {
        let source = r#"
struct Point {
    x: i32,
    y: i32,
}

impl Point {
    fn new(x: i32, y: i32) -> Self {
        Point { x, y }
    }

    fn distance(&self) -> f64 {
        ((self.x as f64).powi(2) + (self.y as f64).powi(2)).sqrt()
    }
}
"#;

        let info = extract_rust(source);
        let point = &info.classes[0];
        let method_names: Vec<&str> = point.methods.iter().map(|m| m.name.as_str()).collect();

        assert!(
            method_names.contains(&"new"),
            "Expected 'new' in methods, got: {:?}",
            method_names
        );
        assert!(
            method_names.contains(&"distance"),
            "Expected 'distance' in methods, got: {:?}",
            method_names
        );
    }

    #[test]
    fn impl_methods_have_is_method_true() {
        let source = r#"
struct Foo {}

impl Foo {
    fn bar(&self) {}
}
"#;

        let info = extract_rust(source);
        let foo = &info.classes[0];
        assert_eq!(foo.methods.len(), 1);
        assert!(
            foo.methods[0].is_method,
            "Methods inside impl blocks should have is_method=true"
        );
    }

    #[test]
    fn impl_method_params_are_extracted() {
        // GIVEN: A method with explicit parameters
        let source = r#"
struct Point {
    x: i32,
    y: i32,
}

impl Point {
    fn new(x: i32, y: i32) -> Self {
        Point { x, y }
    }

    fn distance(&self) -> f64 {
        0.0
    }
}
"#;

        let info = extract_rust(source);
        let point = &info.classes[0];

        let new_method = point
            .methods
            .iter()
            .find(|m| m.name == "new")
            .expect("Should have 'new' method");
        // Params should include "x" and "y" (may or may not include self-like params
        // depending on implementation, but must have the named params)
        assert!(
            new_method.params.contains(&"x".to_string())
                || new_method.params.iter().any(|p| p.contains("x")),
            "new() should have param 'x', got: {:?}",
            new_method.params
        );

        let distance_method = point
            .methods
            .iter()
            .find(|m| m.name == "distance")
            .expect("Should have 'distance' method");
        // &self should appear in params
        assert!(
            distance_method.params.iter().any(|p| p.contains("self")),
            "distance() should have a self-like param, got: {:?}",
            distance_method.params
        );
    }
}

// =============================================================================
// Contract 2: Multiple impl blocks for same struct
// =============================================================================

mod multiple_impl_blocks {
    use super::*;

    #[test]
    fn methods_from_separate_impl_blocks_are_merged() {
        // GIVEN: A struct with two separate impl blocks
        let source = r#"
struct Foo {}

impl Foo {
    fn method_a(&self) {}
}

impl Foo {
    fn method_b(&self) {}
}
"#;

        // WHEN: We extract module info
        let info = extract_rust(source);

        // THEN: Both methods should be in the single ClassInfo
        assert_eq!(
            info.classes.len(),
            1,
            "Should have exactly 1 ClassInfo for Foo"
        );
        let foo = &info.classes[0];
        assert_eq!(foo.name, "Foo");
        assert_eq!(
            foo.methods.len(),
            2,
            "Foo should have 2 methods from 2 impl blocks, got: {:?}",
            foo.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );

        let method_names: Vec<&str> = foo.methods.iter().map(|m| m.name.as_str()).collect();
        assert!(method_names.contains(&"method_a"), "Missing method_a");
        assert!(method_names.contains(&"method_b"), "Missing method_b");
    }

    #[test]
    fn three_impl_blocks_all_merged() {
        let source = r#"
struct Widget {}

impl Widget {
    fn new() -> Self { Widget {} }
}

impl Widget {
    fn render(&self) {}
}

impl Widget {
    fn destroy(self) {}
}
"#;

        let info = extract_rust(source);
        let widget = &info.classes[0];
        assert_eq!(
            widget.methods.len(),
            3,
            "Widget should have 3 methods from 3 impl blocks, got: {:?}",
            widget.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Contract 3: Trait implementation (impl Trait for Struct)
// =============================================================================

mod trait_impl {
    use super::*;

    #[test]
    fn trait_impl_methods_associated_with_struct() {
        // GIVEN: A struct with both inherent and trait impl
        let source = r#"
struct Bar {}

impl Bar {
    fn custom(&self) {}
}

impl std::fmt::Display for Bar {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Bar")
    }
}
"#;

        // WHEN: We extract module info
        let info = extract_rust(source);

        // THEN: Bar should have both custom() and fmt()
        let bar = info
            .classes
            .iter()
            .find(|c| c.name == "Bar")
            .expect("Should have ClassInfo for Bar");

        let method_names: Vec<&str> = bar.methods.iter().map(|m| m.name.as_str()).collect();
        assert!(
            method_names.contains(&"custom"),
            "Bar should have 'custom' from inherent impl, got: {:?}",
            method_names
        );
        assert!(
            method_names.contains(&"fmt"),
            "Bar should have 'fmt' from trait impl, got: {:?}",
            method_names
        );
    }

    #[test]
    fn trait_impl_methods_have_is_method_true() {
        let source = r#"
struct MyType {}

impl Clone for MyType {
    fn clone(&self) -> Self {
        MyType {}
    }
}
"#;

        let info = extract_rust(source);
        let my_type = info
            .classes
            .iter()
            .find(|c| c.name == "MyType")
            .expect("Should have ClassInfo for MyType");

        assert_eq!(my_type.methods.len(), 1, "MyType should have clone method");
        assert!(
            my_type.methods[0].is_method,
            "Trait impl methods should have is_method=true"
        );
    }
}

// =============================================================================
// Contract 4: Enum with impl
// =============================================================================

mod enum_with_impl {
    use super::*;

    #[test]
    fn enum_impl_methods_extracted() {
        // GIVEN: An enum with an impl block
        let source = r##"
enum Color {
    Red,
    Green,
    Blue,
}

impl Color {
    fn to_hex(&self) -> String {
        match self {
            Color::Red => "#FF0000".to_string(),
            Color::Green => "#00FF00".to_string(),
            Color::Blue => "#0000FF".to_string(),
        }
    }

    fn is_warm(&self) -> bool {
        matches!(self, Color::Red)
    }
}
"##;

        // WHEN: We extract module info
        let info = extract_rust(source);

        // THEN: Color should be in classes with its methods
        let color = info
            .classes
            .iter()
            .find(|c| c.name == "Color")
            .expect("Enum 'Color' should appear in classes");
        assert_eq!(
            color.methods.len(),
            2,
            "Color should have 2 methods (to_hex, is_warm), got: {:?}",
            color.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );

        let method_names: Vec<&str> = color.methods.iter().map(|m| m.name.as_str()).collect();
        assert!(method_names.contains(&"to_hex"), "Missing to_hex");
        assert!(method_names.contains(&"is_warm"), "Missing is_warm");
    }

    #[test]
    fn enum_with_data_and_impl() {
        let source = r#"
enum Shape {
    Circle(f64),
    Rectangle(f64, f64),
}

impl Shape {
    fn area(&self) -> f64 {
        match self {
            Shape::Circle(r) => 3.14 * r * r,
            Shape::Rectangle(w, h) => w * h,
        }
    }
}
"#;

        let info = extract_rust(source);
        let shape = info
            .classes
            .iter()
            .find(|c| c.name == "Shape")
            .expect("Enum 'Shape' should appear in classes");
        assert_eq!(
            shape.methods.len(),
            1,
            "Shape should have 1 method (area), got: {:?}",
            shape.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Contract 5: Orphan impl (no matching struct in file)
// =============================================================================

mod orphan_impl {
    use super::*;

    #[test]
    fn orphan_impl_does_not_crash() {
        // GIVEN: An impl block with no matching struct definition
        let source = r#"
impl SomeExternalType {
    fn helper(&self) {}
}
"#;

        // WHEN: We extract module info
        // THEN: Should not panic or error
        let info = extract_rust(source);

        // The method should NOT appear in the top-level functions list
        let func_names: Vec<&str> = info.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(
            !func_names.contains(&"helper"),
            "Orphan impl method 'helper' should NOT be in top-level functions, got: {:?}",
            func_names
        );
    }

    #[test]
    fn trait_impl_for_external_type_does_not_crash() {
        // GIVEN: A trait impl for a type not defined in this file
        let source = r#"
impl std::fmt::Display for i32 {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}
"#;

        // WHEN/THEN: Should not panic
        let info = extract_rust(source);

        // fmt should not leak into top-level functions
        let func_names: Vec<&str> = info.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(
            !func_names.contains(&"fmt"),
            "Trait impl method for external type should NOT be in functions list"
        );
    }
}

// =============================================================================
// Contract 6: Methods NOT in top-level functions
// =============================================================================

mod method_function_separation {
    use super::*;

    #[test]
    fn impl_methods_not_in_top_level_functions() {
        // GIVEN: A file with both free functions and impl methods
        let source = r#"
fn top_level() -> i32 {
    42
}

pub fn public_func(x: i32) -> i32 {
    x * 2
}

async fn async_func() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

struct Animal {
    name: String,
}

impl Animal {
    fn new(name: &str) -> Self {
        Animal { name: name.to_string() }
    }

    fn speak(&self) -> &str {
        &self.name
    }
}

struct Dog {
    breed: String,
}

impl Dog {
    fn fetch(&self) -> &str {
        "ball"
    }

    fn bark(&self) -> &str {
        "woof"
    }
}
"#;

        // WHEN: We extract module info
        let info = extract_rust(source);

        // THEN: Only free functions should be in functions list
        let func_names: Vec<&str> = info.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(
            func_names.contains(&"top_level"),
            "Missing top_level function"
        );
        assert!(
            func_names.contains(&"public_func"),
            "Missing public_func function"
        );
        assert!(
            func_names.contains(&"async_func"),
            "Missing async_func function"
        );

        // Methods should NOT be in functions list
        assert!(
            !func_names.contains(&"new"),
            "'new' (impl method) should NOT be in functions list"
        );
        assert!(
            !func_names.contains(&"speak"),
            "'speak' (impl method) should NOT be in functions list"
        );
        assert!(
            !func_names.contains(&"fetch"),
            "'fetch' (impl method) should NOT be in functions list"
        );
        assert!(
            !func_names.contains(&"bark"),
            "'bark' (impl method) should NOT be in functions list"
        );
    }

    #[test]
    fn free_functions_have_is_method_false() {
        let source = r#"
fn standalone() {}

struct S {}
impl S {
    fn method(&self) {}
}
"#;

        let info = extract_rust(source);
        let standalone = info
            .functions
            .iter()
            .find(|f| f.name == "standalone")
            .expect("Should have standalone function");
        assert!(
            !standalone.is_method,
            "Free function 'standalone' should have is_method=false"
        );
    }
}

// =============================================================================
// Full fixture test (matches test_rust.rs fixture expectations)
// =============================================================================

mod fixture_parity {
    use super::*;

    #[test]
    fn test_rust_fixture_expected_counts() {
        // GIVEN: The standard Rust test fixture
        // Expected: 3f 2c 4m (3 functions, 2 structs/classes, 4 methods)
        let source = r#"
fn top_level() -> i32 {
    42
}

pub fn public_func(x: i32) -> i32 {
    x * 2
}

async fn async_func() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

struct Animal {
    name: String,
}

impl Animal {
    fn new(name: &str) -> Self {
        Animal { name: name.to_string() }
    }

    fn speak(&self) -> &str {
        &self.name
    }
}

struct Dog {
    breed: String,
}

impl Dog {
    fn fetch(&self) -> &str {
        "ball"
    }

    fn bark(&self) -> &str {
        "woof"
    }
}
"#;

        let info = extract_rust(source);

        // 3 top-level functions
        assert_eq!(
            info.functions.len(),
            3,
            "Expected 3 top-level functions, got {}: {:?}",
            info.functions.len(),
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );

        // 2 classes (structs)
        assert_eq!(
            info.classes.len(),
            2,
            "Expected 2 classes (Animal, Dog), got {}: {:?}",
            info.classes.len(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        // Animal has 2 methods
        let animal = info
            .classes
            .iter()
            .find(|c| c.name == "Animal")
            .expect("Should have Animal class");
        assert_eq!(
            animal.methods.len(),
            2,
            "Animal should have 2 methods (new, speak), got {}: {:?}",
            animal.methods.len(),
            animal.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );

        // Dog has 2 methods
        let dog = info
            .classes
            .iter()
            .find(|c| c.name == "Dog")
            .expect("Should have Dog class");
        assert_eq!(
            dog.methods.len(),
            2,
            "Dog should have 2 methods (fetch, bark), got {}: {:?}",
            dog.methods.len(),
            dog.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );

        // Total methods across all classes = 4
        let total_methods: usize = info.classes.iter().map(|c| c.methods.len()).sum();
        assert_eq!(
            total_methods, 4,
            "Expected 4 total methods across all classes, got {}",
            total_methods
        );
    }
}

// =============================================================================
// Edge case: Generic impl blocks
// =============================================================================

mod generic_impl {
    use super::*;

    #[test]
    fn generic_struct_impl_methods_extracted() {
        // GIVEN: A generic struct with a generic impl block
        let source = r#"
struct Container<T> {
    value: T,
}

impl<T> Container<T> {
    fn new(value: T) -> Self {
        Container { value }
    }

    fn get(&self) -> &T {
        &self.value
    }
}
"#;

        // WHEN: We extract module info
        let info = extract_rust(source);

        // THEN: Container should have methods from the generic impl
        let container = info
            .classes
            .iter()
            .find(|c| c.name == "Container")
            .expect("Should have ClassInfo for Container");
        assert_eq!(
            container.methods.len(),
            2,
            "Container should have 2 methods (new, get), got: {:?}",
            container
                .methods
                .iter()
                .map(|m| &m.name)
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Edge case: impl block appears before struct definition
// =============================================================================

mod impl_ordering {
    use super::*;

    #[test]
    fn impl_before_struct_still_associates() {
        // GIVEN: An impl block that appears BEFORE the struct definition
        // (Rust allows this, and the fix should handle it via multi-pass)
        let source = r#"
impl Late {
    fn early_method(&self) {}
}

struct Late {
    data: i32,
}
"#;

        // WHEN: We extract module info
        let info = extract_rust(source);

        // THEN: Late should have its method even though impl came first
        let late = info
            .classes
            .iter()
            .find(|c| c.name == "Late")
            .expect("Should have ClassInfo for Late");
        assert_eq!(
            late.methods.len(),
            1,
            "Late should have 1 method (early_method), got: {:?}",
            late.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }
}
