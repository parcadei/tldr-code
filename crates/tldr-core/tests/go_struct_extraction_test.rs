//! Tests for Go struct/interface extraction and method association (Gap 2)
//!
//! These tests define the TARGET behavior for Go struct extraction as ClassInfo
//! and method-to-receiver association. They are expected to FAIL initially because
//! `extract_classes_detailed` returns empty Vec for Go (line 219 of extract.rs):
//!
//! ```rust,ignore
//! Language::C | Language::Go | Language::Lua | Language::Luau | Language::Ocaml => {} // No classes
//! ```
//!
//! After implementation, all tests should pass and demonstrate:
//! 1. Go structs extracted as ClassInfo
//! 2. Go interfaces extracted as ClassInfo with inline methods
//! 3. Methods associated with receiver structs (pointer and value receivers)
//! 4. Top-level functions separated from methods
//! 5. Orphan methods (no struct in file) auto-vivify ClassInfo
//!
//! # Behavioral Contracts (from gap2-go-struct-spec.md)
//!
//! - BC-1: Struct extraction
//! - BC-2: Interface extraction
//! - BC-3: Method association by receiver type
//! - BC-4: Pointer vs value receiver normalization
//! - BC-5: Methods without matching struct (auto-vivification)
//! - BC-6: Interface methods
//!
//! # Running
//!
//! ```bash
//! cargo test -p tldr-core --test go_struct_extraction_test -- --test-threads=1
//! ```

use std::path::Path;

use tldr_core::ast::extract::extract_from_tree;
use tldr_core::ast::parser::ParserPool;
use tldr_core::types::Language;

// =============================================================================
// Helper: Parse Go source and extract ModuleInfo
// =============================================================================

/// Parse Go source code and return (functions, classes) from extract_from_tree.
/// This is the canonical extraction path used by the rest of the system.
fn extract_go(source: &str) -> (Vec<String>, Vec<(String, Vec<String>)>) {
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Go).expect("Go should parse");
    let module = extract_from_tree(&tree, source, Language::Go, Path::new("test.go"), None)
        .expect("extract_from_tree should succeed");

    // Collect function names
    let func_names: Vec<String> = module.functions.iter().map(|f| f.name.clone()).collect();

    // Collect classes with their method names
    let class_info: Vec<(String, Vec<String>)> = module
        .classes
        .iter()
        .map(|c| {
            let methods: Vec<String> = c.methods.iter().map(|m| m.name.clone()).collect();
            (c.name.clone(), methods)
        })
        .collect();

    (func_names, class_info)
}

// =============================================================================
// BC-1: Basic Struct Extraction
// =============================================================================

/// BC-1: Given `type Server struct { Port int }`, extraction should return
/// ClassInfo with name "Server".
#[test]
fn test_bc1_basic_struct_extraction() {
    let source = r#"
package main

type Server struct {
    Port int
    Host string
}
"#;

    let (_functions, classes) = extract_go(source);

    assert!(
        !classes.is_empty(),
        "BC-1: Go struct should be extracted as ClassInfo, got empty classes"
    );
    assert_eq!(
        classes.len(),
        1,
        "BC-1: Should extract exactly 1 class, got {}",
        classes.len()
    );
    assert_eq!(
        classes[0].0, "Server",
        "BC-1: ClassInfo name should be 'Server', got '{}'",
        classes[0].0
    );
    assert!(
        classes[0].1.is_empty(),
        "BC-1: Server should have no methods (none defined), got {:?}",
        classes[0].1
    );
}

// =============================================================================
// BC-2: Interface Extraction
// =============================================================================

/// BC-2: Given `type Handler interface { Handle() }`, should create ClassInfo
/// with name "Handler" and methods containing "Handle".
#[test]
fn test_bc2_interface_extraction() {
    let source = r#"
package main

type Handler interface {
    Handle() error
    Close()
}
"#;

    let (_functions, classes) = extract_go(source);

    assert!(
        !classes.is_empty(),
        "BC-2: Go interface should be extracted as ClassInfo, got empty classes"
    );
    assert_eq!(
        classes.len(),
        1,
        "BC-2: Should extract exactly 1 class for interface, got {}",
        classes.len()
    );
    assert_eq!(
        classes[0].0, "Handler",
        "BC-2: ClassInfo name should be 'Handler', got '{}'",
        classes[0].0
    );
    assert_eq!(
        classes[0].1.len(),
        2,
        "BC-2: Handler should have 2 methods (Handle, Close), got {:?}",
        classes[0].1
    );
    assert!(
        classes[0].1.contains(&"Handle".to_string()),
        "BC-2: Handler methods should include 'Handle', got {:?}",
        classes[0].1
    );
    assert!(
        classes[0].1.contains(&"Close".to_string()),
        "BC-2: Handler methods should include 'Close', got {:?}",
        classes[0].1
    );
}

// =============================================================================
// BC-3: Method Association by Receiver Type
// =============================================================================

/// BC-3: Given `type Server struct {}` and `func (s *Server) Start() {}`,
/// the "Start" method should be in Server's ClassInfo.methods, NOT in
/// top-level functions.
#[test]
fn test_bc3_method_receiver_association() {
    let source = r#"
package main

type Server struct {
    port int
}

func (s *Server) Start() error {
    return nil
}

func (s *Server) Stop() {
}
"#;

    let (functions, classes) = extract_go(source);

    // Server should exist as ClassInfo
    assert_eq!(
        classes.len(),
        1,
        "BC-3: Should extract exactly 1 class (Server), got {}",
        classes.len()
    );
    assert_eq!(
        classes[0].0, "Server",
        "BC-3: ClassInfo name should be 'Server'"
    );

    // Server should have 2 methods: Start and Stop
    assert_eq!(
        classes[0].1.len(),
        2,
        "BC-3: Server should have 2 methods, got {:?}",
        classes[0].1
    );
    assert!(
        classes[0].1.contains(&"Start".to_string()),
        "BC-3: Server methods should include 'Start', got {:?}",
        classes[0].1
    );
    assert!(
        classes[0].1.contains(&"Stop".to_string()),
        "BC-3: Server methods should include 'Stop', got {:?}",
        classes[0].1
    );

    // Methods should NOT be in the top-level functions list
    assert!(
        !functions.contains(&"Start".to_string()),
        "BC-3: 'Start' should NOT be in top-level functions (it's a method), got functions: {:?}",
        functions
    );
    assert!(
        !functions.contains(&"Stop".to_string()),
        "BC-3: 'Stop' should NOT be in top-level functions (it's a method), got functions: {:?}",
        functions
    );
}

// =============================================================================
// BC-4: Pointer vs Value Receiver Normalization
// =============================================================================

/// BC-4: Both `func (s *Server) Foo()` (pointer receiver) and
/// `func (s Server) Bar()` (value receiver) should associate with "Server".
#[test]
fn test_bc4_pointer_vs_value_receiver() {
    let source = r#"
package main

type Server struct {
    port int
}

func (s *Server) StartPointer() error {
    return nil
}

func (s Server) StopValue() {
}
"#;

    let (_functions, classes) = extract_go(source);

    assert_eq!(
        classes.len(),
        1,
        "BC-4: Should extract exactly 1 class (Server), got {}",
        classes.len()
    );
    assert_eq!(classes[0].0, "Server", "BC-4: ClassInfo should be 'Server'");

    // Both pointer and value receiver methods should be associated
    assert_eq!(
        classes[0].1.len(),
        2,
        "BC-4: Server should have 2 methods (pointer + value receiver), got {:?}",
        classes[0].1
    );
    assert!(
        classes[0].1.contains(&"StartPointer".to_string()),
        "BC-4: Pointer receiver method 'StartPointer' should be in Server.methods, got {:?}",
        classes[0].1
    );
    assert!(
        classes[0].1.contains(&"StopValue".to_string()),
        "BC-4: Value receiver method 'StopValue' should be in Server.methods, got {:?}",
        classes[0].1
    );
}

// =============================================================================
// BC-5: Methods Without Matching Struct (Auto-vivification)
// =============================================================================

/// BC-5: Given `func (s *OrphanType) Method() {}` with no OrphanType struct
/// in the file, should create ClassInfo { name: "OrphanType", methods: [Method] }.
#[test]
fn test_bc5_orphan_method_auto_vivification() {
    let source = r#"
package main

func (s *OrphanType) Method() {
}
"#;

    let (functions, classes) = extract_go(source);

    // Should auto-vivify a ClassInfo for OrphanType
    assert_eq!(
        classes.len(),
        1,
        "BC-5: Should auto-vivify ClassInfo for OrphanType, got {} classes",
        classes.len()
    );
    assert_eq!(
        classes[0].0, "OrphanType",
        "BC-5: Auto-vivified ClassInfo should be named 'OrphanType', got '{}'",
        classes[0].0
    );
    assert_eq!(
        classes[0].1.len(),
        1,
        "BC-5: OrphanType should have 1 method, got {:?}",
        classes[0].1
    );
    assert!(
        classes[0].1.contains(&"Method".to_string()),
        "BC-5: OrphanType methods should include 'Method', got {:?}",
        classes[0].1
    );

    // Method should NOT be in top-level functions
    assert!(
        !functions.contains(&"Method".to_string()),
        "BC-5: 'Method' should NOT be in top-level functions, got {:?}",
        functions
    );
}

// =============================================================================
// BC-6: Interface Methods (multiple methods in interface body)
// =============================================================================

/// BC-6: Given an interface with multiple method specs, all should be extracted
/// as methods in the ClassInfo.
#[test]
fn test_bc6_interface_methods_extracted() {
    let source = r#"
package main

type ReadWriteCloser interface {
    Read(p []byte) (int, error)
    Write(p []byte) (int, error)
    Close() error
}
"#;

    let (_functions, classes) = extract_go(source);

    assert_eq!(
        classes.len(),
        1,
        "BC-6: Should extract 1 class for interface, got {}",
        classes.len()
    );
    assert_eq!(classes[0].0, "ReadWriteCloser");
    assert_eq!(
        classes[0].1.len(),
        3,
        "BC-6: Interface should have 3 methods (Read, Write, Close), got {:?}",
        classes[0].1
    );
    assert!(classes[0].1.contains(&"Read".to_string()));
    assert!(classes[0].1.contains(&"Write".to_string()));
    assert!(classes[0].1.contains(&"Close".to_string()));
}

// =============================================================================
// T4: Functions vs Methods Separation
// =============================================================================

/// T4: Top-level `func main() {}` should be in functions list,
/// `func (s *Server) Start()` should NOT be in functions list.
#[test]
fn test_t4_functions_vs_methods_separation() {
    let source = r#"
package main

func Hello() string {
    return "hi"
}

type Greeter struct {
    prefix string
}

func (g *Greeter) Greet() string {
    return "hello"
}

func main() {
    g := &Greeter{prefix: "Hi"}
    g.Greet()
}
"#;

    let (functions, classes) = extract_go(source);

    // Top-level functions: Hello and main
    assert_eq!(
        functions.len(),
        2,
        "T4: Should have exactly 2 top-level functions (Hello, main), got {:?}",
        functions
    );
    assert!(
        functions.contains(&"Hello".to_string()),
        "T4: Top-level functions should include 'Hello', got {:?}",
        functions
    );
    assert!(
        functions.contains(&"main".to_string()),
        "T4: Top-level functions should include 'main', got {:?}",
        functions
    );

    // Greet should NOT be in top-level functions
    assert!(
        !functions.contains(&"Greet".to_string()),
        "T4: 'Greet' is a method, should NOT be in top-level functions, got {:?}",
        functions
    );

    // Greeter class with Greet method
    assert_eq!(
        classes.len(),
        1,
        "T4: Should extract 1 class (Greeter), got {}",
        classes.len()
    );
    assert_eq!(classes[0].0, "Greeter");
    assert_eq!(
        classes[0].1.len(),
        1,
        "T4: Greeter should have 1 method (Greet), got {:?}",
        classes[0].1
    );
    assert!(classes[0].1.contains(&"Greet".to_string()));
}

// =============================================================================
// T5: Mixed Struct and Interface
// =============================================================================

/// T5: Both struct and interface types should be extracted, with methods
/// correctly associated.
#[test]
fn test_t5_mixed_struct_and_interface() {
    let source = r#"
package main

type Server struct {
    port int
}

type Handler interface {
    Handle()
}

func (s *Server) Start() {
}
"#;

    let (_functions, classes) = extract_go(source);

    assert_eq!(
        classes.len(),
        2,
        "T5: Should extract 2 classes (Server struct + Handler interface), got {}",
        classes.len()
    );

    // Find Server and Handler by name (order may vary)
    let server = classes.iter().find(|(name, _)| name == "Server");
    let handler = classes.iter().find(|(name, _)| name == "Handler");

    assert!(
        server.is_some(),
        "T5: Should find Server in classes, got {:?}",
        classes
    );
    assert!(
        handler.is_some(),
        "T5: Should find Handler in classes, got {:?}",
        classes
    );

    let server = server.unwrap();
    let handler = handler.unwrap();

    // Server should have 1 method (Start)
    assert_eq!(
        server.1.len(),
        1,
        "T5: Server should have 1 method (Start), got {:?}",
        server.1
    );
    assert!(server.1.contains(&"Start".to_string()));

    // Handler should have 1 method (Handle) from interface body
    assert_eq!(
        handler.1.len(),
        1,
        "T5: Handler should have 1 method (Handle), got {:?}",
        handler.1
    );
    assert!(handler.1.contains(&"Handle".to_string()));
}

// =============================================================================
// T6: Multiple Structs with Methods to Correct Receivers
// =============================================================================

/// Given `type A struct {}` with `func (a *A) Foo()` and `type B struct {}`
/// with `func (b *B) Bar()`, methods should associate with correct structs.
#[test]
fn test_t6_multiple_structs_correct_association() {
    let source = r#"
package main

type A struct {
    x int
}

type B struct {
    y string
}

func (a *A) Foo() {
}

func (a *A) Baz() {
}

func (b *B) Bar() {
}
"#;

    let (_functions, classes) = extract_go(source);

    assert_eq!(
        classes.len(),
        2,
        "T6: Should extract 2 classes (A, B), got {}",
        classes.len()
    );

    let class_a = classes.iter().find(|(name, _)| name == "A");
    let class_b = classes.iter().find(|(name, _)| name == "B");

    assert!(class_a.is_some(), "T6: Should find class A");
    assert!(class_b.is_some(), "T6: Should find class B");

    let class_a = class_a.unwrap();
    let class_b = class_b.unwrap();

    // A should have Foo and Baz
    assert_eq!(
        class_a.1.len(),
        2,
        "T6: A should have 2 methods (Foo, Baz), got {:?}",
        class_a.1
    );
    assert!(class_a.1.contains(&"Foo".to_string()));
    assert!(class_a.1.contains(&"Baz".to_string()));

    // B should have Bar only
    assert_eq!(
        class_b.1.len(),
        1,
        "T6: B should have 1 method (Bar), got {:?}",
        class_b.1
    );
    assert!(class_b.1.contains(&"Bar".to_string()));

    // Cross-check: Bar should NOT be in A, Foo should NOT be in B
    assert!(
        !class_a.1.contains(&"Bar".to_string()),
        "T6: A should NOT contain B's method 'Bar'"
    );
    assert!(
        !class_b.1.contains(&"Foo".to_string()),
        "T6: B should NOT contain A's method 'Foo'"
    );
}

// =============================================================================
// T7: is_method flag on extracted methods
// =============================================================================

/// Methods associated with structs should have is_method = true.
#[test]
fn test_t7_method_flag_on_class_methods() {
    let source = r#"
package main

type Server struct {}

func (s *Server) Start() {
}

func Hello() {
}
"#;

    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Go).expect("Go should parse");
    let module = extract_from_tree(&tree, source, Language::Go, Path::new("test.go"), None)
        .expect("extract_from_tree should succeed");

    // Check that Server's Start method has is_method = true
    assert_eq!(module.classes.len(), 1, "Should have 1 class (Server)");
    let server = &module.classes[0];
    assert_eq!(server.name, "Server");
    assert_eq!(server.methods.len(), 1);
    assert!(
        server.methods[0].is_method,
        "T7: Server.Start should have is_method = true"
    );

    // Check that Hello function does NOT have is_method = true
    let hello = module.functions.iter().find(|f| f.name == "Hello");
    assert!(hello.is_some(), "T7: Should find Hello in functions");
    assert!(
        !hello.unwrap().is_method,
        "T7: Top-level function Hello should have is_method = false"
    );
}

// =============================================================================
// T8: Struct line number
// =============================================================================

/// ClassInfo should have the correct line number for the struct declaration.
#[test]
fn test_t8_struct_line_number() {
    let source = r#"package main

type Server struct {
    port int
}
"#;

    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Go).expect("Go should parse");
    let module = extract_from_tree(&tree, source, Language::Go, Path::new("test.go"), None)
        .expect("extract_from_tree should succeed");

    assert_eq!(module.classes.len(), 1, "Should have 1 class");
    // The type declaration `type Server struct {` is on line 3 (1-based)
    assert!(
        module.classes[0].line_number > 0,
        "T8: ClassInfo should have a non-zero line number, got {}",
        module.classes[0].line_number
    );
}

// =============================================================================
// E5: Multiple Type Specs in One Declaration Block
// =============================================================================

/// Grouped type declarations: `type ( A struct{} \n B struct{} )`
/// should extract both A and B as separate ClassInfo entries.
#[test]
fn test_e5_grouped_type_declaration() {
    let source = r#"
package main

type (
    Server struct {
        port int
    }
    Client struct {
        addr string
    }
)
"#;

    let (_functions, classes) = extract_go(source);

    assert_eq!(
        classes.len(),
        2,
        "E5: Should extract both Server and Client from grouped type decl, got {}",
        classes.len()
    );

    let names: Vec<&str> = classes.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"Server"),
        "E5: Should find Server in grouped decl, got {:?}",
        names
    );
    assert!(
        names.contains(&"Client"),
        "E5: Should find Client in grouped decl, got {:?}",
        names
    );
}

// =============================================================================
// Integration: Full Go file with structs, interfaces, methods, and functions
// =============================================================================

/// Comprehensive test using the SAMPLE_GO_CODE pattern from language_parity_test.
/// This matches the existing sample that has Greeter struct with Greet method.
#[test]
fn test_full_go_file_extraction() {
    let source = r#"
package main

import (
    "fmt"
)

func Hello(name string) string {
    return fmt.Sprintf("Hello, %s!", name)
}

type Greeter struct {
    Prefix string
}

func (g *Greeter) Greet(name string) string {
    return fmt.Sprintf("%s %s", g.Prefix, name)
}

func main() {
    fmt.Println(Hello("World"))
}
"#;

    let (functions, classes) = extract_go(source);

    // Functions: Hello and main (top-level only)
    assert!(
        functions.contains(&"Hello".to_string()),
        "Full: 'Hello' should be in top-level functions, got {:?}",
        functions
    );
    assert!(
        functions.contains(&"main".to_string()),
        "Full: 'main' should be in top-level functions, got {:?}",
        functions
    );
    assert!(
        !functions.contains(&"Greet".to_string()),
        "Full: 'Greet' should NOT be in top-level functions (it's a method), got {:?}",
        functions
    );

    // Classes: Greeter with Greet method
    assert_eq!(
        classes.len(),
        1,
        "Full: Should extract 1 class (Greeter), got {}",
        classes.len()
    );
    assert_eq!(classes[0].0, "Greeter");
    assert_eq!(
        classes[0].1.len(),
        1,
        "Full: Greeter should have 1 method (Greet), got {:?}",
        classes[0].1
    );
    assert!(classes[0].1.contains(&"Greet".to_string()));
}

// =============================================================================
// Regression guard: existing Go function extraction should not break
// =============================================================================

/// Existing behavior: extract_functions should still find top-level function
/// names. This test guards against regressions when modifying Go extraction.
#[test]
fn test_regression_go_functions_still_extracted() {
    let source = r#"
package main

import "fmt"

func Hello(name string) string {
    return fmt.Sprintf("Hello, %s!", name)
}

func main() {
    fmt.Println(Hello("World"))
}
"#;

    let (functions, _classes) = extract_go(source);

    assert!(
        !functions.is_empty(),
        "Regression: Go functions should still be extracted"
    );
    assert!(
        functions.contains(&"Hello".to_string()),
        "Regression: Should find 'Hello' function, got {:?}",
        functions
    );
    assert!(
        functions.contains(&"main".to_string()),
        "Regression: Should find 'main' function, got {:?}",
        functions
    );
}
