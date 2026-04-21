//! Discriminative hypothesis tests for Tier-2 Fowler smell data availability.
//!
//! These tests validate whether the existing call graph infrastructure populates
//! the data fields needed by Tier-2 smell detectors (Feature Envy, Inappropriate
//! Intimacy, Middle Man, Refused Bequest).
//!
//! NO new detection functions are implemented here. We only call existing APIs
//! and measure what data they produce.

use std::path::Path;

use super::cross_file_types::{CallSite, CallType, FuncDef, VarType, FileIR};
use super::var_types::extract_python_definitions;
use super::resolution::apply_type_resolution;
use crate::types::Language;

// =============================================================================
// H1: receiver_type is populated for method calls
// =============================================================================

/// H1a: After parsing, method calls have receiver but NOT receiver_type yet.
/// The scanner (extract_python_definitions) creates CallSites with receiver=Some
/// but receiver_type=None. This is expected -- receiver_type is filled later
/// by the resolution pass (apply_type_resolution).
#[test]
fn h1a_scanner_creates_method_calls_with_receiver() {
    let source = r#"
class User:
    def save(self):
        pass

class Order:
    def calculate(self):
        pass

def process():
    user = User()
    user.save()
    order = Order()
    order.calculate()
"#;

    let result = extract_python_definitions(source, Path::new("test.py"));

    // Collect all calls
    let all_calls: Vec<&CallSite> = result.calls.values().flat_map(|v| v.iter()).collect();

    let method_calls: Vec<&CallSite> = all_calls
        .iter()
        .filter(|c| c.call_type == CallType::Method)
        .copied()
        .collect();

    // We expect method calls for user.save() and order.calculate()
    assert!(
        method_calls.len() >= 2,
        "Expected at least 2 method calls, got {}. All calls: {:?}",
        method_calls.len(),
        all_calls
    );

    // All method calls should have receiver populated
    let with_receiver = method_calls.iter().filter(|c| c.receiver.is_some()).count();
    assert_eq!(
        with_receiver,
        method_calls.len(),
        "All method calls should have receiver field populated"
    );

    // At scan time, receiver_type should be None (populated later by resolution)
    let with_receiver_type_at_scan = method_calls
        .iter()
        .filter(|c| c.receiver_type.is_some())
        .count();
    eprintln!(
        "H1a: {}/{} method calls have receiver_type at scan time (expected 0)",
        with_receiver_type_at_scan,
        method_calls.len()
    );
}

/// H1b: After apply_type_resolution, receiver_type gets populated via VarType lookup.
/// This is the critical test: does the resolution pass fill in receiver_type
/// from constructor assignments like `user = User()`?
#[test]
fn h1b_resolution_populates_receiver_type_from_var_types() {
    let source = r#"
class User:
    def save(self):
        pass

class Order:
    def calculate(self):
        pass

def process():
    user = User()
    user.save()
    order = Order()
    order.calculate()
"#;

    let parse_result = extract_python_definitions(source, Path::new("test.py"));

    // Build a FileIR from the parse result
    let mut file_ir = FileIR::new("test.py".into());
    file_ir.funcs = parse_result.funcs;
    file_ir.classes = parse_result.classes;
    file_ir.imports = parse_result.imports;
    file_ir.calls = parse_result.calls;
    file_ir.var_types = parse_result.var_types;

    // Apply type resolution
    apply_type_resolution(&mut file_ir, source, Language::Python);

    // Collect all method calls after resolution
    let all_calls: Vec<&CallSite> = file_ir.calls.values().flat_map(|v| v.iter()).collect();
    let method_calls: Vec<&CallSite> = all_calls
        .iter()
        .filter(|c| c.call_type == CallType::Method)
        .copied()
        .collect();

    let with_receiver_type = method_calls
        .iter()
        .filter(|c| c.receiver_type.is_some())
        .count();
    let total_method = method_calls.len();

    eprintln!("H1b: {}/{} method calls have receiver_type after resolution", with_receiver_type, total_method);

    for call in &method_calls {
        eprintln!(
            "  call: {}.{} | receiver={:?} | receiver_type={:?}",
            call.caller, call.target, call.receiver, call.receiver_type
        );
    }

    // CRITICAL ASSERTION: At least some method calls should have receiver_type
    // If this fails, Feature Envy and Inappropriate Intimacy cannot work.
    assert!(
        with_receiver_type > 0,
        "FALSIFIED: No method calls have receiver_type after resolution. \
         Feature Envy / Inappropriate Intimacy will be blind. \
         Total method calls: {}, with receiver_type: {}",
        total_method,
        with_receiver_type
    );

    // Ideal: most or all method calls from constructor assignments should resolve
    let ratio = with_receiver_type as f64 / total_method.max(1) as f64;
    eprintln!(
        "H1b: receiver_type population ratio: {:.1}% ({}/{})",
        ratio * 100.0,
        with_receiver_type,
        total_method
    );
}

/// H1c: self.method() calls get receiver_type set to the enclosing class name.
#[test]
fn h1c_self_calls_get_receiver_type() {
    let source = r#"
class Calculator:
    def add(self, x, y):
        return x + y

    def compute(self):
        result = self.add(1, 2)
        return result
"#;

    let parse_result = extract_python_definitions(source, Path::new("test.py"));
    let mut file_ir = FileIR::new("test.py".into());
    file_ir.funcs = parse_result.funcs;
    file_ir.classes = parse_result.classes;
    file_ir.imports = parse_result.imports;
    file_ir.calls = parse_result.calls;
    file_ir.var_types = parse_result.var_types;

    apply_type_resolution(&mut file_ir, source, Language::Python);

    let all_calls: Vec<&CallSite> = file_ir.calls.values().flat_map(|v| v.iter()).collect();
    let self_calls: Vec<&CallSite> = all_calls
        .iter()
        .filter(|c| c.receiver.as_deref() == Some("self"))
        .copied()
        .collect();

    eprintln!("H1c: Found {} self.X() calls", self_calls.len());
    for call in &self_calls {
        eprintln!(
            "  self.{} -> receiver_type={:?}",
            call.target, call.receiver_type
        );
    }

    // self.add() should resolve to receiver_type=Calculator
    let resolved_self = self_calls
        .iter()
        .filter(|c| c.receiver_type.is_some())
        .count();
    assert!(
        resolved_self > 0,
        "self.method() calls should have receiver_type set to enclosing class"
    );

    // Check it resolves to Calculator specifically
    let calc_calls: Vec<&&CallSite> = self_calls
        .iter()
        .filter(|c| c.receiver_type.as_deref() == Some("Calculator"))
        .collect();
    eprintln!("H1c: {} self calls resolved to Calculator", calc_calls.len());
}

// =============================================================================
// H2: InheritanceReport contains real parent-child data
// (tested via extract_inheritance which requires files on disk)
// =============================================================================

/// H2: Test that ClassDef.bases is populated from Python source.
/// This tests the call graph layer's class extraction, not the full
/// inheritance module (which requires files on disk).
#[test]
fn h2_classdef_bases_populated() {
    let source = r#"
class Animal:
    def speak(self):
        pass

class Dog(Animal):
    def speak(self):
        return "Woof"

class GuideDog(Dog, Animal):
    def guide(self):
        pass
"#;

    let result = extract_python_definitions(source, Path::new("test.py"));

    eprintln!("H2: Found {} classes", result.classes.len());
    for class in &result.classes {
        eprintln!(
            "  class {} | bases={:?} | methods={:?}",
            class.name, class.bases, class.methods
        );
    }

    // Animal should have no bases
    let animal = result.classes.iter().find(|c| c.name == "Animal");
    assert!(animal.is_some(), "Animal class not found");
    assert!(
        animal.unwrap().bases.is_empty(),
        "Animal should have no bases"
    );

    // Dog should inherit from Animal
    let dog = result.classes.iter().find(|c| c.name == "Dog");
    assert!(dog.is_some(), "Dog class not found");
    let dog = dog.unwrap();
    assert!(
        dog.bases.contains(&"Animal".to_string()),
        "Dog should have Animal as base. Actual bases: {:?}",
        dog.bases
    );

    // GuideDog should have both Dog and Animal
    let guide_dog = result.classes.iter().find(|c| c.name == "GuideDog");
    assert!(guide_dog.is_some(), "GuideDog class not found");
    let guide_dog = guide_dog.unwrap();
    assert!(
        guide_dog.bases.contains(&"Dog".to_string()),
        "GuideDog should have Dog as base"
    );
    assert!(
        guide_dog.bases.contains(&"Animal".to_string()),
        "GuideDog should have Animal as base"
    );
}

/// H2b: InheritanceReport from file-based analysis.
/// Uses the full inheritance module with temp files.
#[test]
fn h2b_inheritance_report_from_file() {
    use crate::inheritance::{extract_inheritance, InheritanceOptions};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let code = r#"
class Parent:
    def parent_method(self):
        pass

class Child(Parent):
    def child_method(self):
        pass

class GrandChild(Child):
    def gc_method(self):
        pass
"#;
    std::fs::write(dir.path().join("classes.py"), code).unwrap();

    let options = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::Python), &options).unwrap();

    eprintln!("H2b: InheritanceReport nodes={}, edges={}", report.nodes.len(), report.edges.len());
    for edge in &report.edges {
        eprintln!(
            "  edge: {} -> {} (kind={:?}, resolution={:?})",
            edge.child, edge.parent, edge.kind, edge.resolution
        );
    }
    for node in &report.nodes {
        eprintln!(
            "  node: {} | bases={:?}",
            node.name, node.bases
        );
    }

    // Should find all 3 classes
    assert_eq!(report.count, 3, "Should find 3 classes");
    assert_eq!(report.nodes.len(), 3, "Should have 3 nodes");

    // Should have Child->Parent and GrandChild->Child edges
    assert!(
        report.edges.len() >= 2,
        "Should have at least 2 edges. Got: {}",
        report.edges.len()
    );

    let child_parent = report
        .edges
        .iter()
        .find(|e| e.child == "Child" && e.parent == "Parent");
    assert!(
        child_parent.is_some(),
        "Child -> Parent edge should exist"
    );

    let gc_child = report
        .edges
        .iter()
        .find(|e| e.child == "GrandChild" && e.parent == "Child");
    assert!(
        gc_child.is_some(),
        "GrandChild -> Child edge should exist"
    );

    // Check nodes have bases populated
    let child_node = report.nodes.iter().find(|n| n.name == "Child");
    assert!(child_node.is_some(), "Child node should exist");
    assert!(
        child_node.unwrap().bases.contains(&"Parent".to_string()),
        "Child node should have Parent in bases"
    );
}

// =============================================================================
// H3: ClassDef.methods is populated for class methods
// =============================================================================

/// H3: ClassDef.methods contains all method names; FuncDef.class_name is populated.
#[test]
fn h3_classdef_methods_and_funcdef_class_name() {
    let source = r#"
class UserService:
    def __init__(self):
        self.users = []

    def add_user(self, user):
        self.users.append(user)

    def remove_user(self, user):
        self.users.remove(user)

    def get_all_users(self):
        return self.users
"#;

    let result = extract_python_definitions(source, Path::new("test.py"));

    eprintln!("H3: Found {} classes, {} functions", result.classes.len(), result.funcs.len());

    // Check ClassDef.methods
    let user_service = result.classes.iter().find(|c| c.name == "UserService");
    assert!(user_service.is_some(), "UserService class not found");
    let user_service = user_service.unwrap();

    eprintln!("H3: UserService.methods = {:?}", user_service.methods);

    let expected_methods = vec!["__init__", "add_user", "remove_user", "get_all_users"];
    for method_name in &expected_methods {
        assert!(
            user_service.methods.contains(&method_name.to_string()),
            "ClassDef.methods should contain '{}'. Actual: {:?}",
            method_name,
            user_service.methods
        );
    }

    assert_eq!(
        user_service.methods.len(),
        expected_methods.len(),
        "ClassDef.methods should have exactly {} methods. Actual: {:?}",
        expected_methods.len(),
        user_service.methods
    );

    // Check FuncDef.class_name for each method
    let method_funcs: Vec<&FuncDef> = result
        .funcs
        .iter()
        .filter(|f| f.class_name.as_deref() == Some("UserService"))
        .collect();

    eprintln!("H3: {} FuncDefs have class_name=UserService", method_funcs.len());
    for func in &method_funcs {
        eprintln!(
            "  func: {} | is_method={} | class_name={:?}",
            func.name, func.is_method, func.class_name
        );
    }

    assert_eq!(
        method_funcs.len(),
        expected_methods.len(),
        "Should have {} FuncDefs with class_name=UserService",
        expected_methods.len()
    );

    // All should have is_method=true
    for func in &method_funcs {
        assert!(
            func.is_method,
            "FuncDef '{}' should have is_method=true",
            func.name
        );
    }
}

// =============================================================================
// H4: VarType tracks self.field patterns
// =============================================================================

/// H4a: VarType entries include constructor assignments.
/// Feature Envy needs to know which variables hold which types.
#[test]
fn h4a_vartype_constructor_assignments() {
    let source = r#"
class User:
    pass

class Order:
    pass

def process():
    user = User()
    order = Order()
    data = {}
    items = []
    name = "Alice"
    count = 42
"#;

    let result = extract_python_definitions(source, Path::new("test.py"));

    eprintln!("H4a: Found {} VarType entries", result.var_types.len());
    for vt in &result.var_types {
        eprintln!(
            "  var={} type={} source={} line={} scope={:?}",
            vt.var_name, vt.type_name, vt.source, vt.line, vt.scope
        );
    }

    // Should have VarType for user = User()
    let user_vt = result
        .var_types
        .iter()
        .find(|vt| vt.var_name == "user" && vt.type_name == "User");
    assert!(
        user_vt.is_some(),
        "VarType for 'user = User()' should exist. All VarTypes: {:?}",
        result.var_types
    );
    assert_eq!(user_vt.unwrap().source, "assignment");

    // Should have VarType for order = Order()
    let order_vt = result
        .var_types
        .iter()
        .find(|vt| vt.var_name == "order" && vt.type_name == "Order");
    assert!(
        order_vt.is_some(),
        "VarType for 'order = Order()' should exist"
    );

    // Should have VarType for dict literal
    let data_vt = result
        .var_types
        .iter()
        .find(|vt| vt.var_name == "data" && vt.type_name == "dict");
    assert!(
        data_vt.is_some(),
        "VarType for 'data = {{}}' should exist"
    );

    // Should have VarType for list literal
    let items_vt = result
        .var_types
        .iter()
        .find(|vt| vt.var_name == "items" && vt.type_name == "list");
    assert!(
        items_vt.is_some(),
        "VarType for 'items = []' should exist"
    );
}

/// H4b: VarType tracks parameter annotations.
#[test]
fn h4b_vartype_parameter_annotations() {
    let source = r#"
class User:
    pass

def process(user: User, name: str, count: int):
    user.save()
"#;

    let result = extract_python_definitions(source, Path::new("test.py"));

    eprintln!("H4b: Found {} VarType entries", result.var_types.len());
    for vt in &result.var_types {
        eprintln!(
            "  var={} type={} source={} scope={:?}",
            vt.var_name, vt.type_name, vt.source, vt.scope
        );
    }

    // Should have parameter type for user: User
    let user_param = result
        .var_types
        .iter()
        .find(|vt| vt.var_name == "user" && vt.type_name == "User" && vt.source == "parameter");
    assert!(
        user_param.is_some(),
        "VarType for parameter 'user: User' should exist"
    );

    // Scope should be the function name
    assert_eq!(
        user_param.unwrap().scope.as_deref(),
        Some("process"),
        "Parameter VarType scope should be the function name"
    );
}

/// H4c: self.field pattern detection (tests whether VarType captures self.x = X() patterns).
/// Note: The current var_types extraction only captures simple identifier LHS, not
/// self.x patterns. This test measures whether that's the case.
#[test]
fn h4c_self_field_patterns() {
    let source = r#"
class Service:
    def __init__(self):
        self.repo = Repository()
        self.cache = Cache()
        self.logger = Logger()
        name = "default"
"#;

    let result = extract_python_definitions(source, Path::new("test.py"));

    eprintln!("H4c: Found {} VarType entries", result.var_types.len());
    for vt in &result.var_types {
        eprintln!(
            "  var={} type={} source={} scope={:?}",
            vt.var_name, vt.type_name, vt.source, vt.scope
        );
    }

    // Check if self.repo, self.cache, self.logger are captured
    let self_field_vts: Vec<&VarType> = result
        .var_types
        .iter()
        .filter(|vt| vt.var_name.starts_with("self."))
        .collect();

    eprintln!(
        "H4c: {} VarType entries have self.field pattern (out of {} total)",
        self_field_vts.len(),
        result.var_types.len()
    );

    // This test is INFORMATIONAL -- it measures whether self.field is tracked.
    // If self_field_vts is empty, it means the Feature Envy detector would
    // need a separate mechanism to distinguish own fields from foreign fields.
    if self_field_vts.is_empty() {
        eprintln!(
            "H4c RESULT: self.field patterns NOT captured in VarType. \
             Feature Envy will need to use receiver=='self' heuristic instead."
        );
    } else {
        eprintln!("H4c RESULT: self.field patterns ARE captured in VarType.");
    }

    // Check that simple assignment IS captured (name = "default")
    // This is the baseline -- simple identifiers should always work.
    let name_vt = result
        .var_types
        .iter()
        .find(|vt| vt.var_name == "name");
    eprintln!("H4c: Simple var 'name' captured: {}", name_vt.is_some());
}

// =============================================================================
// H5: Call graph tracks delegation patterns
// =============================================================================

/// H5: A pure delegation method (single forwarding call) is visible in the call graph.
/// Middle Man detection needs to see that a method's body contains only one call
/// to another object's method.
#[test]
fn h5_delegation_pattern_visible() {
    let source = r#"
class RealService:
    def do_thing(self):
        return "done"

    def do_other(self):
        return "other"

class Proxy:
    def __init__(self):
        self.delegate = RealService()

    def do_thing(self):
        return self.delegate.do_thing()

    def do_other(self):
        return self.delegate.do_other()

    def do_complex(self):
        a = self.delegate.do_thing()
        b = self.delegate.do_other()
        return a + b
"#;

    let result = extract_python_definitions(source, Path::new("test.py"));

    eprintln!("H5: Calls map has {} entries", result.calls.len());
    for (caller, calls) in &result.calls {
        eprintln!("  caller '{}' has {} calls:", caller, calls.len());
        for call in calls {
            eprintln!(
                "    -> {}.{} (type={:?}, receiver={:?}, receiver_type={:?})",
                call.caller, call.target, call.call_type, call.receiver, call.receiver_type
            );
        }
    }

    // Proxy.do_thing should have exactly 1 call (delegation)
    let proxy_do_thing_calls = result.calls.get("Proxy.do_thing");
    assert!(
        proxy_do_thing_calls.is_some(),
        "Proxy.do_thing should have calls. Available callers: {:?}",
        result.calls.keys().collect::<Vec<_>>()
    );
    let proxy_do_thing_calls = proxy_do_thing_calls.unwrap();

    // Filter to actual method/function calls (not Direct calls to builtins)
    let method_calls: Vec<&CallSite> = proxy_do_thing_calls
        .iter()
        .filter(|c| matches!(c.call_type, CallType::Method | CallType::Attr))
        .collect();

    eprintln!(
        "H5: Proxy.do_thing has {} method calls (delegation detection needs exactly 1)",
        method_calls.len()
    );

    assert_eq!(
        method_calls.len(),
        1,
        "Proxy.do_thing should have exactly 1 method call (pure delegation). Got: {:?}",
        method_calls
    );

    // The call should be to delegate.do_thing
    let delegation_call = method_calls[0];
    assert_eq!(delegation_call.target, "do_thing");
    // Receiver should be self.delegate -- let's check what the scanner produces
    eprintln!(
        "H5: Delegation call receiver = {:?} (need to check if this includes 'self.delegate' or just 'delegate')",
        delegation_call.receiver
    );

    // Proxy.do_complex should have 2 calls (NOT pure delegation)
    let proxy_do_complex = result.calls.get("Proxy.do_complex");
    assert!(proxy_do_complex.is_some(), "Proxy.do_complex should have calls");
    let complex_method_calls: Vec<&CallSite> = proxy_do_complex
        .unwrap()
        .iter()
        .filter(|c| matches!(c.call_type, CallType::Method | CallType::Attr))
        .collect();
    eprintln!(
        "H5: Proxy.do_complex has {} method calls (should be 2, not pure delegation)",
        complex_method_calls.len()
    );
    assert!(
        complex_method_calls.len() > 1,
        "Proxy.do_complex should have multiple calls (not pure delegation)"
    );
}

/// H5b: After resolution, delegation calls get receiver_type populated.
/// This lets Middle Man detector know WHICH class the delegation goes to.
#[test]
fn h5b_delegation_receiver_type_resolution() {
    let source = r#"
class RealService:
    def do_thing(self):
        return "done"

class Proxy:
    def __init__(self):
        self.delegate = RealService()

    def do_thing(self):
        return self.delegate.do_thing()
"#;

    let parse_result = extract_python_definitions(source, Path::new("test.py"));
    let mut file_ir = FileIR::new("test.py".into());
    file_ir.funcs = parse_result.funcs;
    file_ir.classes = parse_result.classes;
    file_ir.imports = parse_result.imports;
    file_ir.calls = parse_result.calls;
    file_ir.var_types = parse_result.var_types;

    apply_type_resolution(&mut file_ir, source, Language::Python);

    // Check Proxy.do_thing delegation call
    let proxy_calls = file_ir.calls.get("Proxy.do_thing");
    assert!(proxy_calls.is_some(), "Proxy.do_thing should have calls");

    let method_calls: Vec<&CallSite> = proxy_calls
        .unwrap()
        .iter()
        .filter(|c| c.call_type == CallType::Method)
        .collect();

    eprintln!("H5b: Proxy.do_thing method calls after resolution:");
    for call in &method_calls {
        eprintln!(
            "  -> target={} receiver={:?} receiver_type={:?}",
            call.target, call.receiver, call.receiver_type
        );
    }

    // Check if any delegation call got receiver_type resolved
    let with_type = method_calls
        .iter()
        .filter(|c| c.receiver_type.is_some())
        .count();
    eprintln!(
        "H5b: {}/{} delegation calls have receiver_type after resolution",
        with_type,
        method_calls.len()
    );

    // This is informational -- if receiver is "self.delegate", the type resolver
    // may or may not resolve it. If it doesn't, Middle Man would need to use
    // self.delegate's VarType (captured in __init__) separately.
}

// =============================================================================
// Summary test: runs all hypotheses and prints combined report
// =============================================================================

/// Combined report of all hypothesis results.
/// Run with: cargo test -p tldr-core hypotheses_summary -- --nocapture
#[test]
fn hypotheses_summary() {
    eprintln!("\n=== Discriminative Hypothesis Summary ===\n");
    eprintln!("H1: receiver_type population");
    eprintln!("  H1a: Scanner creates Method calls with receiver (tested above)");
    eprintln!("  H1b: Resolution populates receiver_type from VarType (tested above)");
    eprintln!("  H1c: self.method() gets receiver_type=EnclosingClass (tested above)");
    eprintln!();
    eprintln!("H2: Inheritance data");
    eprintln!("  H2a: ClassDef.bases populated from source (tested above)");
    eprintln!("  H2b: InheritanceReport with real edges (tested above)");
    eprintln!();
    eprintln!("H3: ClassDef.methods populated");
    eprintln!("  ClassDef.methods + FuncDef.class_name (tested above)");
    eprintln!();
    eprintln!("H4: VarType tracking");
    eprintln!("  H4a: Constructor assignments tracked (tested above)");
    eprintln!("  H4b: Parameter annotations tracked (tested above)");
    eprintln!("  H4c: self.field patterns (tested above - informational)");
    eprintln!();
    eprintln!("H5: Delegation patterns");
    eprintln!("  H5a: Single-call method visible in call graph (tested above)");
    eprintln!("  H5b: Delegation receiver_type resolution (tested above)");
    eprintln!("\n=== End Summary ===\n");
}
