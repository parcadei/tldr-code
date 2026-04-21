//! TDD Integration Tests: VarType Injection into Call Graph Resolution
//!
//! These tests verify that `build_project_call_graph_v2` correctly uses
//! `FileIR.var_types` to set `CallSite.receiver_type` and thereby resolve
//! method calls via type-aware dispatch.
//!
//! **Failing tests are expected to FAIL** until:
//! 1. Language handlers populate `FileIR.var_types` from tree-sitter AST
//! 2. `apply_type_resolution()` in `builder_v2.rs` reads `file_ir.var_types`
//!    and injects them into `CallSite.receiver_type`
//!
//! Root cause: `apply_type_resolution()` (builder_v2.rs:2236) ignores
//! `file_ir.var_types` entirely. The `VarType` struct exists, the field exists
//! on `FileIR`, but no language handler populates it and
//! `apply_type_resolution()` never reads it. The regex-based
//! `resolve_receiver_type()` handles simple `var = Type()` patterns by scanning
//! source text backwards, but has fundamental limitations that VarType
//! injection would fix.

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use tldr_core::callgraph::{build_project_call_graph_v2, BuildConfig, CrossFileCallEdge};

// =============================================================================
// Helpers
// =============================================================================

/// Create a Python file in a directory, creating parent dirs as needed.
fn write_py_file(root: &Path, relative_path: &str, content: &str) {
    let full_path = root.join(relative_path);
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent).expect("Failed to create parent directories");
    }
    fs::write(&full_path, content).expect("Failed to write Python file");
}

/// Check if an edge exists matching src_func -> dst_func pattern.
fn has_edge(edges: &[CrossFileCallEdge], src_func: &str, dst_func: &str) -> bool {
    edges
        .iter()
        .any(|e| e.src_func.contains(src_func) && e.dst_func == dst_func)
}

/// Print all edges for debugging test failures.
fn dump_edges(edges: &[CrossFileCallEdge]) {
    eprintln!("=== Call graph edges ({} total) ===", edges.len());
    for edge in edges {
        eprintln!(
            "  {} ({}) -> {} ({}), type={:?}, via={:?}",
            edge.src_func,
            edge.src_file.display(),
            edge.dst_func,
            edge.dst_file.display(),
            edge.call_type,
            edge.via_import
        );
    }
    eprintln!("=== End edges ===");
}

/// Build a call graph with type resolution enabled.
fn build_with_type_resolution(root: &Path) -> tldr_core::callgraph::CallGraphIR {
    let config = BuildConfig {
        language: "python".to_string(),
        use_type_resolution: true,
        respect_ignore: false,
        ..Default::default()
    };
    build_project_call_graph_v2(root, config).expect("Call graph build should succeed")
}

// =============================================================================
// Test 1: VarType extraction -- constructor assignments
// =============================================================================
//
// PREREQUISITE: Language handlers must populate FileIR.var_types.
// This is the foundational test. Without var_types populated, the entire
// VarType injection pipeline is dead code.
//
// Tests that `h = Helper()` produces:
//   VarType { var_name: "h", type_name: "Helper", source: "assignment",
//             line: N, scope: Some("main") }
//
// EXPECTED STATUS: FAILS

#[test]
fn test_python_handler_populates_var_types_from_constructor() {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let root = dir.path();

    write_py_file(
        root,
        "test_vartypes.py",
        r#"
class Helper:
    def compute(self):
        pass

class Processor:
    def run(self):
        pass

def main():
    h = Helper()
    p = Processor()
    h.compute()
    p.run()
"#,
    );

    let ir = build_with_type_resolution(root);

    let file_ir = ir
        .files
        .values()
        .find(|f| f.path.to_string_lossy().contains("test_vartypes"))
        .expect("Should find test_vartypes.py in IR");

    eprintln!("var_types for test_vartypes.py: {:?}", file_ir.var_types);

    // --- Assertion: var_types is non-empty ---
    assert!(
        !file_ir.var_types.is_empty(),
        "FileIR.var_types should be populated by the Python language handler. \
         Expected VarType entries for `h = Helper()` and `p = Processor()`. \
         Currently no language handler populates var_types -- this is the root \
         cause of the VarType injection gap."
    );

    // --- Assertion: h = Helper() extracted ---
    let h_vt = file_ir.var_types.iter().find(|vt| vt.var_name == "h");
    assert!(h_vt.is_some(), "Expected VarType for `h = Helper()`");
    let h_vt = h_vt.unwrap();
    assert_eq!(h_vt.type_name, "Helper");
    assert_eq!(h_vt.source, "assignment");
    assert_eq!(h_vt.scope, Some("main".to_string()));

    // --- Assertion: p = Processor() extracted ---
    let p_vt = file_ir.var_types.iter().find(|vt| vt.var_name == "p");
    assert!(p_vt.is_some(), "Expected VarType for `p = Processor()`");
    let p_vt = p_vt.unwrap();
    assert_eq!(p_vt.type_name, "Processor");
    assert_eq!(p_vt.source, "assignment");
    assert_eq!(p_vt.scope, Some("main".to_string()));
}

// =============================================================================
// Test 2: VarType extraction -- parameter type annotations
// =============================================================================
//
// PREREQUISITE: Language handlers must extract parameter annotations.
// Tests that `def handle(svc: Service)` produces:
//   VarType { var_name: "svc", type_name: "Service", source: "parameter",
//             line: N, scope: Some("handle") }
//
// EXPECTED STATUS: FAILS

#[test]
fn test_python_handler_populates_var_types_from_annotations() {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let root = dir.path();

    write_py_file(
        root,
        "service.py",
        r#"
class Service:
    def execute(self):
        pass

    def rollback(self):
        pass
"#,
    );

    write_py_file(
        root,
        "handler.py",
        r#"
from service import Service

def handle(svc: Service):
    svc.execute()
    svc.rollback()
"#,
    );

    let ir = build_with_type_resolution(root);

    let handler_ir = ir
        .files
        .values()
        .find(|f| f.path.to_string_lossy().contains("handler"))
        .expect("Should find handler.py in IR");

    eprintln!("var_types for handler.py: {:?}", handler_ir.var_types);

    // --- Assertion: svc: Service extracted ---
    let svc_vt = handler_ir.var_types.iter().find(|vt| vt.var_name == "svc");
    assert!(
        svc_vt.is_some(),
        "Expected VarType for parameter `svc: Service` in handle(). \
         The Python handler should extract type annotations on parameters \
         as VarType entries with source='parameter'."
    );
    let svc_vt = svc_vt.unwrap();
    assert_eq!(svc_vt.type_name, "Service");
    assert_eq!(svc_vt.source, "parameter");
    assert_eq!(svc_vt.scope, Some("handle".to_string()));
}

// =============================================================================
// Test 3: Ambiguous method resolution via VarType
// =============================================================================
//
// SCENARIO: Two classes (Sender, Receiver) both have a `send()` method.
// A function creates one via constructor and calls send(). Without VarType,
// the resolver cannot determine which class's send() is called because:
// - Strategy 8 (global unique name) fails: "send" is not unique
// - The constructor `Sender()` happens on a different line than the call
//
// WITH VarType injection: `obj = Sender()` produces VarType{type_name:"Sender"},
// which gets injected as receiver_type. Strategy 0 then resolves correctly.
//
// This test currently PASSES because the regex-based resolver scans backward
// and finds `obj = Sender()`, setting receiver_type = "Sender". However,
// we include it as a regression guard: when VarType injection replaces the
// regex, this must still work.
//
// EXPECTED STATUS: PASSES (regex resolver handles this; serves as regression guard)

#[test]
fn test_ambiguous_method_resolved_via_constructor_type() {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let root = dir.path();

    write_py_file(
        root,
        "comm.py",
        r#"
class Sender:
    def send(self):
        pass

class Receiver:
    def send(self):
        pass

def dispatch():
    obj = Sender()
    obj.send()
"#,
    );

    let ir = build_with_type_resolution(root);
    let edges = ir.edges();
    dump_edges(edges);

    // obj = Sender(), so obj.send() must resolve to Sender.send, NOT Receiver.send
    assert!(
        has_edge(edges, "dispatch", "Sender.send"),
        "Expected edge dispatch -> Sender.send. Variable `obj = Sender()` should \
         set receiver_type='Sender', resolving obj.send() to Sender.send."
    );
    assert!(
        !has_edge(edges, "dispatch", "Receiver.send"),
        "dispatch -> Receiver.send should NOT exist. `obj` is a Sender, not Receiver."
    );
}

// =============================================================================
// Test 4: Non-regression -- type resolution must not decrease edge count
// =============================================================================
//
// Running with `use_type_resolution: true` must produce >= edges as
// `use_type_resolution: false`. Type resolution is additive only.
//
// EXPECTED STATUS: PASSES

#[test]
fn test_type_resolution_does_not_decrease_edge_count() {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let root = dir.path();

    write_py_file(
        root,
        "models.py",
        r#"
class User:
    def save(self):
        pass

    def validate(self):
        pass

class Admin(User):
    def promote(self):
        pass
"#,
    );

    write_py_file(
        root,
        "utils.py",
        r#"
def helper():
    pass

def format_name(name):
    return name.strip()
"#,
    );

    write_py_file(
        root,
        "main.py",
        r#"
from models import User, Admin
from utils import helper, format_name

def process():
    u = User()
    u.save()
    u.validate()
    helper()
    format_name("test")

def admin_flow():
    a = Admin()
    a.promote()
    a.save()
"#,
    );

    // Run WITHOUT type resolution
    let config_off = BuildConfig {
        language: "python".to_string(),
        use_type_resolution: false,
        respect_ignore: false,
        ..Default::default()
    };
    let ir_off = build_project_call_graph_v2(root, config_off)
        .expect("Call graph build (type_resolution=off) should succeed");
    let edges_off = ir_off.edge_count();

    // Run WITH type resolution
    let config_on = BuildConfig {
        language: "python".to_string(),
        use_type_resolution: true,
        respect_ignore: false,
        ..Default::default()
    };
    let ir_on = build_project_call_graph_v2(root, config_on)
        .expect("Call graph build (type_resolution=on) should succeed");
    let edges_on = ir_on.edge_count();

    eprintln!(
        "Edge count: type_resolution=off -> {}, type_resolution=on -> {}",
        edges_off, edges_on
    );

    assert!(
        edges_on >= edges_off,
        "Type resolution must NOT reduce edge count. \
         Got edges_off={} but edges_on={}. \
         This violates the non-regression contract (Spec Contract 1).",
        edges_off,
        edges_on
    );
}
