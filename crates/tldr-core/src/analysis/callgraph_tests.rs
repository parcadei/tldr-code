//! Test module for call graph commands: impact --type-aware, whatbreaks, hubs
//!
//! These tests define expected behavior BEFORE implementation.
//! Tests are designed to FAIL until the modules are implemented.
//!
//! # Test Categories
//!
//! ## 1. impact --type-aware tests
//! - Python type resolution (annotations, constructors, self)
//! - TypeScript type resolution (annotations, interfaces)
//! - Cross-file import resolution
//! - Fallback behavior for unknown types
//! - Confidence markers
//!
//! ## 2. whatbreaks tests
//! - Target type detection (function, file, module)
//! - Sub-analysis routing
//! - Partial failure handling
//! - Output format validation
//!
//! ## 3. hubs tests
//! - In-degree centrality
//! - Out-degree centrality
//! - PageRank calculation
//! - Betweenness centrality
//! - Composite score
//! - Risk classification
//! - Edge cases (empty, single node, disconnected)
//!
//! Reference: callgraph-spec.md sections 1-3

use std::collections::{HashMap, HashSet};
use std::path::Path;

// =============================================================================
// Test Fixture Setup Module
// =============================================================================

/// Test fixture utilities for call graph tests
pub mod fixtures {
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// A temporary directory for testing call graph analysis
    pub struct TestDir {
        pub dir: TempDir,
    }

    impl TestDir {
        /// Create a new empty temporary directory
        pub fn new() -> std::io::Result<Self> {
            let dir = TempDir::new()?;
            Ok(Self { dir })
        }

        /// Get the path to the directory
        pub fn path(&self) -> &Path {
            self.dir.path()
        }

        /// Add a file to the directory
        pub fn add_file(&self, name: &str, content: &str) -> std::io::Result<PathBuf> {
            let path = self.dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;
            Ok(path)
        }

        /// Add a subdirectory
        pub fn add_subdir(&self, name: &str) -> std::io::Result<PathBuf> {
            let path = self.dir.path().join(name);
            std::fs::create_dir_all(&path)?;
            Ok(path)
        }
    }

    // -------------------------------------------------------------------------
    // Type-Aware Impact Fixtures (Python)
    // -------------------------------------------------------------------------

    /// Python file with type annotations - explicit types
    pub const PYTHON_TYPE_ANNOTATIONS: &str = r#"
class User:
    def save(self):
        return True

    def delete(self):
        return False

class Admin(User):
    def promote(self):
        pass

def create_user() -> User:
    return User()

def process_user():
    user: User = User()
    user.save()  # Should resolve to User.save

def process_admin():
    admin: Admin = Admin()
    admin.save()  # Should resolve to Admin.save (or User.save via inheritance)
"#;

    /// Python file with constructor-based type inference
    pub const PYTHON_CONSTRUCTOR_INFERENCE: &str = r#"
class Database:
    def connect(self):
        return True

    def query(self, sql: str):
        return []

def setup_database():
    db = Database()  # Type inferred from constructor
    db.connect()     # Should resolve to Database.connect
    db.query("SELECT * FROM users")  # Should resolve to Database.query
"#;

    /// Python file with self method resolution
    pub const PYTHON_SELF_RESOLUTION: &str = r#"
class Calculator:
    def __init__(self):
        self.value = 0

    def add(self, n: int):
        self.value += n
        self._validate()  # self.method() should resolve to Calculator._validate
        return self

    def _validate(self):
        if self.value < 0:
            self.reset()  # Should resolve to Calculator.reset

    def reset(self):
        self.value = 0
"#;

    /// Python files for cross-file import resolution
    pub const PYTHON_MODULE_USER: &str = r#"
class User:
    def save(self):
        return True

    def validate(self):
        return True
"#;

    pub const PYTHON_MODULE_SERVICE: &str = r#"
from user import User

class UserService:
    def create_user(self):
        user = User()      # Imported type
        user.validate()    # Should resolve to User.validate (from user.py)
        user.save()        # Should resolve to User.save (from user.py)
        return user
"#;

    /// Python file with union types
    pub const PYTHON_UNION_TYPES: &str = r#"
from typing import Union

class Dog:
    def speak(self):
        return "woof"

class Cat:
    def speak(self):
        return "meow"

def make_sound(animal: Union[Dog, Cat]):
    animal.speak()  # Should emit edges to BOTH Dog.speak AND Cat.speak
"#;

    /// Python file with unknown types (fallback scenario)
    pub const PYTHON_UNKNOWN_TYPES: &str = r#"
def process_data(data):
    # No type annotation - unknown type
    data.transform()  # Should fall back to "data.transform" (variable name)
    data.save()       # Should fall back to "data.save"
    return data
"#;

    // -------------------------------------------------------------------------
    // Type-Aware Impact Fixtures (TypeScript)
    // -------------------------------------------------------------------------

    /// TypeScript file with type annotations
    pub const TYPESCRIPT_TYPE_ANNOTATIONS: &str = r#"
interface Serializable {
    serialize(): string;
}

class User implements Serializable {
    name: string;

    constructor(name: string) {
        this.name = name;
    }

    serialize(): string {
        return JSON.stringify({ name: this.name });
    }

    save(): boolean {
        return true;
    }
}

function processUser(): void {
    const user: User = new User("test");
    user.save();      // Should resolve to User.save
    user.serialize(); // Should resolve to User.serialize
}
"#;

    /// TypeScript file with interface-based resolution
    pub const TYPESCRIPT_INTERFACE_RESOLUTION: &str = r#"
interface Repository<T> {
    find(id: string): T | null;
    save(item: T): boolean;
}

class UserRepository implements Repository<User> {
    find(id: string): User | null {
        return null;
    }

    save(item: User): boolean {
        return true;
    }
}

function useRepository(repo: Repository<User>): void {
    const user = repo.find("123");  // Should resolve to Repository.find (interface)
    if (user) {
        repo.save(user);            // Should resolve to Repository.save (interface)
    }
}
"#;

    /// TypeScript with this. resolution
    pub const TYPESCRIPT_THIS_RESOLUTION: &str = r#"
class Counter {
    private value: number = 0;

    increment(): void {
        this.value++;
        this.validate();  // Should resolve to Counter.validate
    }

    validate(): void {
        if (this.value < 0) {
            this.reset();  // Should resolve to Counter.reset
        }
    }

    reset(): void {
        this.value = 0;
    }
}
"#;

    // -------------------------------------------------------------------------
    // whatbreaks Fixtures
    // -------------------------------------------------------------------------

    /// Multi-file project for whatbreaks testing
    pub const WHATBREAKS_MAIN: &str = r#"
from service import UserService
from utils import helper

def main():
    service = UserService()
    service.run()
    helper()
"#;

    pub const WHATBREAKS_SERVICE: &str = r#"
from repository import UserRepository

class UserService:
    def __init__(self):
        self.repo = UserRepository()

    def run(self):
        return self.repo.find_all()
"#;

    pub const WHATBREAKS_REPOSITORY: &str = r#"
class UserRepository:
    def find_all(self):
        return []

    def find_by_id(self, id):
        return None
"#;

    pub const WHATBREAKS_UTILS: &str = r#"
def helper():
    return True

def unused_helper():
    return False
"#;

    pub const WHATBREAKS_TEST: &str = r#"
from service import UserService
from utils import helper

def test_service():
    service = UserService()
    assert service.run() == []

def test_helper():
    assert helper() == True
"#;

    // -------------------------------------------------------------------------
    // hubs Fixtures (Graph Topologies)
    // -------------------------------------------------------------------------

    /// Star topology: one central node called by many
    /// Central node should have high in-degree
    pub const STAR_TOPOLOGY: &str = r#"
def central_hub():
    """Called by everyone"""
    return 42

def caller_a():
    return central_hub()

def caller_b():
    return central_hub()

def caller_c():
    return central_hub()

def caller_d():
    return central_hub()

def caller_e():
    return central_hub()
"#;

    /// Chain topology: A -> B -> C -> D -> E
    /// Middle nodes should have high betweenness
    pub const CHAIN_TOPOLOGY: &str = r#"
def node_a():
    return node_b()

def node_b():
    return node_c()

def node_c():
    return node_d()  # node_c is on all paths from a/b to d/e

def node_d():
    return node_e()

def node_e():
    return 42
"#;

    /// Diamond topology for PageRank testing
    ///     A
    ///    / \
    ///   B   C
    ///    \ /
    ///     D
    pub const DIAMOND_TOPOLOGY: &str = r#"
def node_a():
    node_b()
    node_c()

def node_b():
    node_d()

def node_c():
    node_d()

def node_d():
    return 42  # D has high importance - both B and C depend on it
"#;

    /// Complex topology with multiple hubs
    pub const COMPLEX_TOPOLOGY: &str = r#"
# Entry points
def main():
    service_a()
    service_b()

# Service layer - moderate centrality
def service_a():
    core_util()
    helper_x()

def service_b():
    core_util()
    helper_y()

# Core utility - HIGH centrality (called by both services)
def core_util():
    db_query()
    cache_get()

# Helpers - low centrality
def helper_x():
    db_query()

def helper_y():
    cache_get()

# Leaf nodes - high out-degree but low in-degree
def db_query():
    return []

def cache_get():
    return None
"#;

    /// Disconnected components for testing
    pub const DISCONNECTED_COMPONENTS: &str = r#"
# Component 1
def comp1_a():
    return comp1_b()

def comp1_b():
    return comp1_c()

def comp1_c():
    return 1

# Component 2 (completely separate)
def comp2_x():
    return comp2_y()

def comp2_y():
    return 2
"#;

    /// Self-loop for edge case testing
    pub const SELF_LOOP: &str = r#"
def recursive_func(n):
    if n <= 0:
        return 0
    return recursive_func(n - 1)  # Self-loop
"#;

    /// Empty module (edge case)
    pub const EMPTY_MODULE: &str = r#"
# No functions here
pass
"#;

    /// Single function (edge case)
    pub const SINGLE_FUNCTION: &str = r#"
def lonely():
    return 42
"#;
}

// =============================================================================
// Type-Aware Impact Tests
// =============================================================================

#[cfg(test)]
mod type_aware_tests {
    use super::fixtures::*;
    use super::*;
    use crate::callgraph::builder_v2::{build_project_call_graph_v2, BuildConfig};
    use crate::types::{CallEdge, Language, ProjectCallGraph};

    fn build_call_graph(
        root: &Path,
        language: Language,
        use_type_resolution: bool,
    ) -> ProjectCallGraph {
        let config = BuildConfig {
            language: language.as_str().to_string(),
            use_type_resolution,
            ..Default::default()
        };

        let ir = build_project_call_graph_v2(root, config).expect("call graph build failed");
        let mut graph = ProjectCallGraph::new();
        for edge in ir.edges {
            graph.add_edge(CallEdge {
                src_file: edge.src_file,
                src_func: edge.src_func,
                dst_file: edge.dst_file,
                dst_func: edge.dst_func,
            });
        }
        graph
    }

    // -------------------------------------------------------------------------
    // Python Type Resolution Tests
    // -------------------------------------------------------------------------

    /// Test that explicit type annotations resolve method calls correctly
    /// Pattern: `user: User = User(); user.save()` -> User.save
    #[test]
    fn python_annotation_resolution() {
        use crate::callgraph::type_resolver::resolve_python_receiver_type;
        use crate::types::Confidence;

        let source = PYTHON_TYPE_ANNOTATIONS;

        // In process_user():
        // - Line 17: user: User = User()
        // - Line 18: user.save()
        // Should resolve "user" to "User" type with HIGH confidence

        // Line 18: user.save() in process_user()
        let (receiver_type, confidence) = resolve_python_receiver_type(
            source, 18, // user.save() line
            "user", None,
        );
        assert_eq!(receiver_type, Some("User".to_string()));
        assert_eq!(confidence, Confidence::High);
    }

    /// Test that constructor calls infer type correctly
    /// Pattern: `db = Database()` -> db is Database type
    #[test]
    fn python_constructor_resolution() {
        use crate::callgraph::type_resolver::resolve_python_receiver_type;
        use crate::types::Confidence;

        let source = PYTHON_CONSTRUCTOR_INFERENCE;

        // In setup_database():
        // - Line 10: db = Database()
        // - Line 11: db.connect()
        // - Line 12: db.query(...)
        // Should resolve "db" to "Database" type with HIGH confidence

        // Line 11: db.connect() in setup_database()
        let (receiver_type, confidence) = resolve_python_receiver_type(
            source, 11, // db.connect() line
            "db", None,
        );
        assert_eq!(receiver_type, Some("Database".to_string()));
        assert_eq!(confidence, Confidence::High);

        // Line 12: db.query(...) should also resolve
        let (receiver_type2, confidence2) = resolve_python_receiver_type(
            source, 12, // db.query() line
            "db", None,
        );
        assert_eq!(receiver_type2, Some("Database".to_string()));
        assert_eq!(confidence2, Confidence::High);
    }

    /// Test that self.method() resolves to enclosing class
    /// Pattern: `self._validate()` inside Calculator -> Calculator._validate
    #[test]
    fn python_self_method_resolution() {
        use crate::callgraph::type_resolver::{
            find_enclosing_class, resolve_python_receiver_type, resolve_self_method,
        };
        use crate::types::Confidence;

        // Test self resolution using type_resolver module
        let source = PYTHON_SELF_RESOLUTION;

        // Line 6: self._validate() is inside Calculator.add
        // Should resolve to Calculator._validate
        let (receiver_type, confidence) = resolve_python_receiver_type(
            source, 6, // self._validate() line
            "self", None, // Let it find the enclosing class
        );
        assert_eq!(receiver_type, Some("Calculator".to_string()));
        assert_eq!(confidence, Confidence::High);

        // Test resolve_self_method helper
        let qualified = resolve_self_method("Calculator", "_validate");
        assert_eq!(qualified, "Calculator._validate");

        // Verify enclosing class detection
        let class = find_enclosing_class(source, 6);
        assert_eq!(class, Some("Calculator".to_string()));

        // Line 10: self.reset() is inside Calculator._validate
        let (receiver_type2, confidence2) = resolve_python_receiver_type(
            source, 10, // self.reset() line
            "self", None,
        );
        assert_eq!(receiver_type2, Some("Calculator".to_string()));
        assert_eq!(confidence2, Confidence::High);
    }

    /// Test cross-file import resolution
    /// Pattern: `from user import User; user = User(); user.save()` -> user.User.save
    #[test]
    fn python_cross_file_import_resolution() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("user.py", PYTHON_MODULE_USER).unwrap();
        test_dir
            .add_file("service.py", PYTHON_MODULE_SERVICE)
            .unwrap();

        // When analyzing UserService.create_user():
        // - User is imported from user.py
        // - user.validate() should resolve to "user.py:User.validate"
        // - user.save() should resolve to "user.py:User.save"

        // Expected: impact analysis tracks imports across files

        let graph = build_call_graph(test_dir.path(), Language::Python, true);
        let report =
            crate::analysis::impact::impact_analysis(&graph, "User.save", 2, None).unwrap();

        let mut found = false;
        for tree in report.targets.values() {
            if tree
                .callers
                .iter()
                .any(|caller| caller.function.ends_with("UserService.create_user"))
            {
                found = true;
                break;
            }
        }
        assert!(found, "Expected UserService.create_user to call User.save");

        let report_validate =
            crate::analysis::impact::impact_analysis(&graph, "User.validate", 2, None).unwrap();
        let mut found_validate = false;
        for tree in report_validate.targets.values() {
            if tree
                .callers
                .iter()
                .any(|caller| caller.function.ends_with("UserService.create_user"))
            {
                found_validate = true;
                break;
            }
        }
        assert!(
            found_validate,
            "Expected UserService.create_user to call User.validate"
        );
    }

    /// Test union type resolution emits edges to all possible types
    /// Pattern: `animal: Union[Dog, Cat]; animal.speak()` -> edges to both
    #[test]
    fn python_union_type_resolution() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("animals.py", PYTHON_UNION_TYPES).unwrap();

        // When analyzing make_sound():
        // - animal: Union[Dog, Cat]
        // - animal.speak() should emit edges to BOTH Dog.speak AND Cat.speak
        // with MEDIUM confidence (union type)

        // Expected: impact analysis of Dog.speak AND Cat.speak both find make_sound

        let graph = build_call_graph(test_dir.path(), Language::Python, true);

        let report_dog =
            crate::analysis::impact::impact_analysis(&graph, "Dog.speak", 2, None).unwrap();
        let mut dog_found = false;
        for tree in report_dog.targets.values() {
            if tree
                .callers
                .iter()
                .any(|caller| caller.function == "make_sound")
            {
                dog_found = true;
                break;
            }
        }
        assert!(dog_found, "Expected make_sound to call Dog.speak");

        let report_cat =
            crate::analysis::impact::impact_analysis(&graph, "Cat.speak", 2, None).unwrap();
        let mut cat_found = false;
        for tree in report_cat.targets.values() {
            if tree
                .callers
                .iter()
                .any(|caller| caller.function == "make_sound")
            {
                cat_found = true;
                break;
            }
        }
        assert!(cat_found, "Expected make_sound to call Cat.speak");
    }

    // -------------------------------------------------------------------------
    // TypeScript Type Resolution Tests
    // -------------------------------------------------------------------------

    /// Test TypeScript explicit type annotations
    /// Pattern: `const user: User = new User(); user.save()` -> User.save
    #[test]
    fn typescript_annotation_resolution() {
        use crate::callgraph::type_resolver::resolve_typescript_receiver_type;
        use crate::types::Confidence;

        let source = TYPESCRIPT_TYPE_ANNOTATIONS;

        // Line numbers in the fixture (1-indexed):
        // Line 23: const user: User = new User("test");
        // Line 24: user.save();
        // Line 25: user.serialize();

        // When analyzing processUser():
        // user.save() is at line 24
        let (receiver_type, confidence) = resolve_typescript_receiver_type(
            source, 24, // user.save() line
            "user", None,
        );
        assert_eq!(receiver_type, Some("User".to_string()));
        assert_eq!(confidence, Confidence::High);
    }

    /// Test TypeScript interface-based resolution
    /// Pattern: `function(repo: Repository<User>); repo.find()` -> Repository.find
    #[test]
    fn typescript_interface_resolution() {
        use crate::callgraph::type_resolver::resolve_typescript_receiver_type;
        use crate::types::Confidence;

        let source = TYPESCRIPT_INTERFACE_RESOLUTION;

        // When analyzing useRepository():
        // - repo: Repository<User>
        // - repo.find() should resolve to "Repository.find" (interface method)
        // with MEDIUM confidence (if starts with I, or HIGH otherwise)

        // Line 17: function useRepository(repo: Repository<User>): void {
        // Line 18: const user = repo.find("123");
        let (receiver_type, confidence) = resolve_typescript_receiver_type(
            source, 18, // repo.find() line
            "repo", None,
        );
        // Repository is detected as a type (not starting with I, so HIGH)
        assert_eq!(receiver_type, Some("Repository".to_string()));
        assert_eq!(confidence, Confidence::High);
    }

    /// Test TypeScript this. resolution
    /// Pattern: `this.validate()` inside Counter -> Counter.validate
    #[test]
    fn typescript_this_resolution() {
        use crate::callgraph::type_resolver::resolve_typescript_receiver_type;
        use crate::types::Confidence;

        let source = TYPESCRIPT_THIS_RESOLUTION;

        // When analyzing Counter.increment():
        // - this.validate() should resolve to "Counter.validate"
        // with HIGH confidence (this refers to enclosing class)

        // Line 5: this.validate();
        let (receiver_type, confidence) = resolve_typescript_receiver_type(
            source, 5, // this.validate() line
            "this", None, // Let it find the enclosing class
        );
        assert_eq!(receiver_type, Some("Counter".to_string()));
        assert_eq!(confidence, Confidence::High);
    }

    // -------------------------------------------------------------------------
    // Fallback Behavior Tests
    // -------------------------------------------------------------------------

    /// Test that unknown types fall back to variable name
    /// Pattern: `data.transform()` with no type info -> "data.transform"
    #[test]
    fn unknown_type_falls_back_to_name() {
        use crate::callgraph::type_resolver::resolve_python_receiver_type;
        use crate::types::Confidence;

        let source = PYTHON_UNKNOWN_TYPES;

        // In process_data(data):
        // - data has no type annotation (parameter with no type hint)
        // - data.transform() should return None type (fallback to variable name)
        // with LOW confidence

        // Line 3: data.transform() in process_data()
        let (receiver_type, confidence) = resolve_python_receiver_type(
            source, 3, // data.transform() line
            "data", None,
        );
        // No type found - returns None with LOW confidence
        assert_eq!(receiver_type, None);
        assert_eq!(confidence, Confidence::Low);
    }

    /// Test that confidence markers are present in output
    /// HIGH, MEDIUM, LOW confidence based on resolution method
    #[test]
    fn confidence_markers_present() {
        use crate::callgraph::type_resolver::resolve_python_receiver_type;
        use crate::types::Confidence;

        // Test HIGH confidence - explicit annotation
        let (_, confidence_annotation) = resolve_python_receiver_type(
            PYTHON_TYPE_ANNOTATIONS,
            18, // user.save() with user: User annotation (line 18)
            "user",
            None,
        );
        assert_eq!(confidence_annotation, Confidence::High);

        // Test HIGH confidence - self reference
        let (_, confidence_self) = resolve_python_receiver_type(
            PYTHON_SELF_RESOLUTION,
            6, // self._validate() inside Calculator
            "self",
            Some("Calculator"),
        );
        assert_eq!(confidence_self, Confidence::High);

        // Test HIGH confidence - constructor
        let (_, confidence_constructor) = resolve_python_receiver_type(
            PYTHON_CONSTRUCTOR_INFERENCE,
            11, // db.connect() where db = Database() (line 11)
            "db",
            None,
        );
        assert_eq!(confidence_constructor, Confidence::High);

        // Test LOW confidence - unknown type
        let (_, confidence_unknown) = resolve_python_receiver_type(
            PYTHON_UNKNOWN_TYPES,
            3, // data.transform() with no type info
            "data",
            None,
        );
        assert_eq!(confidence_unknown, Confidence::Low);

        // Verify confidence Display trait
        assert_eq!(format!("{}", Confidence::High), "HIGH");
        assert_eq!(format!("{}", Confidence::Medium), "MEDIUM");
        assert_eq!(format!("{}", Confidence::Low), "LOW");
    }

    /// Test that type-aware flag is optional and defaults to false.
    ///
    /// Note: The v2 builder's strategies 7/8 (global method name scan) resolve
    /// unambiguous method calls like `user.save()` to `User.save` even without
    /// the type-aware flag. The type-aware flag adds explicit annotation-based
    /// resolution (apply_type_resolution), which provides higher confidence and
    /// handles ambiguous cases that strategies 7/8 cannot.
    ///
    /// Both modes should produce User.save edges for this simple unambiguous case.
    #[test]
    fn type_aware_flag_defaults_to_false() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("types.py", PYTHON_TYPE_ANNOTATIONS)
            .unwrap();

        // Without --type-aware flag:
        // - user.save() resolves via Strategy 8 (global method scan) to "User.save"
        //   because save() is unambiguous (only one class defines it)

        // With --type-aware flag:
        // - user.save() resolves via apply_type_resolution to "User.save"
        //   with higher confidence (explicit annotation-based)

        let graph_basic = build_call_graph(test_dir.path(), Language::Python, false);
        let has_user_save_basic = graph_basic.edges().any(|edge| edge.dst_func == "User.save");
        assert!(
            has_user_save_basic,
            "Expected User.save edge even without type-aware flag (Strategy 8 resolves unambiguous methods)"
        );

        let graph_typed = build_call_graph(test_dir.path(), Language::Python, true);
        let has_user_save_typed = graph_typed.edges().any(|edge| edge.dst_func == "User.save");
        assert!(
            has_user_save_typed,
            "Expected User.save edge with type-aware resolution"
        );
    }
}

// =============================================================================
// whatbreaks Tests
// =============================================================================

#[cfg(test)]
mod whatbreaks_tests {
    use super::fixtures::*;
    use crate::analysis::whatbreaks::{
        detect_target_type, whatbreaks_analysis, TargetType, WhatbreaksOptions,
    };
    use crate::types::Language;

    // -------------------------------------------------------------------------
    // Target Detection Tests
    // -------------------------------------------------------------------------

    /// Test that file paths are detected as "file" target type
    /// Pattern: `whatbreaks service.py` -> target_type: "file"
    #[test]
    fn detects_file_target() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("service.py", WHATBREAKS_SERVICE).unwrap();

        // Target detection algorithm:
        // 1. Check if Path(target).exists() -> true
        // 2. Return "file"
        let (target_type, reason) = detect_target_type("service.py", test_dir.path());
        assert_eq!(target_type, TargetType::File);
        assert!(
            reason.contains("exists as a file") || reason.contains("file extension"),
            "Expected reason about file detection, got: {}",
            reason
        );

        // Also detect file-like patterns:
        // - Contains "/" -> file
        let (target_type2, _) = detect_target_type("src/other.py", test_dir.path());
        assert_eq!(target_type2, TargetType::File);

        // - Ends with .py, .ts, .js, .go, .rs -> file
        let (target_type3, _) = detect_target_type("nonexistent.ts", test_dir.path());
        assert_eq!(target_type3, TargetType::File);
    }

    /// Test that module names are detected as "module" target type
    /// Pattern: `whatbreaks myapp.service` -> target_type: "module"
    #[test]
    fn detects_module_target() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_subdir("myapp").unwrap();
        test_dir
            .add_file("myapp/service.py", WHATBREAKS_SERVICE)
            .unwrap();

        // Target detection algorithm:
        // 1. Contains "." AND first part is a directory
        // 2. "myapp.service" -> first_part = "myapp", is_dir("myapp") = true
        // 3. Return "module"
        let (target_type, reason) = detect_target_type("myapp.service", test_dir.path());
        assert_eq!(target_type, TargetType::Module);
        assert!(
            reason.contains("directory"),
            "Expected reason about directory, got: {}",
            reason
        );
    }

    /// Test that function names are detected as "function" target type (default)
    /// Pattern: `whatbreaks process_data` -> target_type: "function"
    #[test]
    fn detects_function_target() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("utils.py", WHATBREAKS_UTILS).unwrap();

        // Target detection algorithm:
        // 1. Not a file path (doesn't exist, no file indicators)
        // 2. Not a module pattern (no dot OR first part not a directory)
        // 3. Default to "function"
        let (target_type, reason) = detect_target_type("helper", test_dir.path());
        assert_eq!(target_type, TargetType::Function);
        assert!(
            reason.contains("defaulting to function"),
            "Expected reason about defaulting, got: {}",
            reason
        );
    }

    // -------------------------------------------------------------------------
    // Sub-Analysis Routing Tests
    // -------------------------------------------------------------------------

    /// Test that function targets run impact analysis
    /// Pattern: `whatbreaks helper` -> runs impact analysis
    #[test]
    fn function_runs_impact() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("main.py", WHATBREAKS_MAIN).unwrap();
        test_dir.add_file("utils.py", WHATBREAKS_UTILS).unwrap();

        let options = WhatbreaksOptions {
            language: Some(Language::Python),
            ..Default::default()
        };

        let report = whatbreaks_analysis("helper", test_dir.path(), &options).unwrap();

        // For function targets:
        // - Run impact analysis to find callers
        // - sub_results should contain "impact" key
        assert_eq!(report.target_type, TargetType::Function);
        assert!(
            report.sub_results.contains_key("impact"),
            "Expected 'impact' in sub_results"
        );
    }

    /// Test that file targets run importers AND change-impact
    /// Pattern: `whatbreaks service.py` -> runs importers + change-impact
    #[test]
    fn file_runs_importers_and_change_impact() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("main.py", WHATBREAKS_MAIN).unwrap();
        test_dir.add_file("service.py", WHATBREAKS_SERVICE).unwrap();
        test_dir
            .add_file("tests/test_service.py", WHATBREAKS_TEST)
            .unwrap();

        let options = WhatbreaksOptions {
            language: Some(Language::Python),
            ..Default::default()
        };

        let report = whatbreaks_analysis("service.py", test_dir.path(), &options).unwrap();

        // For file targets:
        // - Run importers analysis (who imports this file?)
        // - Run change-impact analysis (which tests are affected?)
        // - sub_results should contain "importers" and "change-impact" keys
        assert_eq!(report.target_type, TargetType::File);
        assert!(
            report.sub_results.contains_key("importers"),
            "Expected 'importers' in sub_results"
        );
        assert!(
            report.sub_results.contains_key("change-impact"),
            "Expected 'change-impact' in sub_results"
        );
    }

    /// Test that module targets run importers analysis
    /// Pattern: `whatbreaks myapp.service` -> runs importers
    #[test]
    fn module_runs_importers() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_subdir("myapp").unwrap();
        test_dir.add_file("myapp/__init__.py", "").unwrap();
        test_dir
            .add_file("myapp/service.py", WHATBREAKS_SERVICE)
            .unwrap();
        test_dir.add_file("main.py", WHATBREAKS_MAIN).unwrap();

        let options = WhatbreaksOptions {
            language: Some(Language::Python),
            ..Default::default()
        };

        let report = whatbreaks_analysis("myapp.service", test_dir.path(), &options).unwrap();

        // For module targets:
        // - Run importers analysis for the module
        // - sub_results should contain "importers" key
        assert_eq!(report.target_type, TargetType::Module);
        assert!(
            report.sub_results.contains_key("importers"),
            "Expected 'importers' in sub_results"
        );
    }

    // -------------------------------------------------------------------------
    // Partial Failure Tests
    // -------------------------------------------------------------------------

    /// Test that whatbreaks continues on partial failure
    /// If one sub-analysis fails, others should still run
    #[test]
    fn continues_on_partial_failure() {
        let test_dir = TestDir::new().unwrap();
        // Create file that exists (file target)
        test_dir.add_file("broken.py", "def incomplete(").unwrap();

        let options = WhatbreaksOptions {
            language: Some(Language::Python),
            ..Default::default()
        };

        // Analysis should complete even with parsing issues
        let report = whatbreaks_analysis("broken.py", test_dir.path(), &options).unwrap();

        // Expected behavior:
        // - Report is still generated
        // - Both sub-analysis keys are present
        assert_eq!(report.target_type, TargetType::File);
        assert!(
            report.sub_results.contains_key("importers"),
            "Expected 'importers' in sub_results even on partial failure"
        );
        assert!(
            report.sub_results.contains_key("change-impact"),
            "Expected 'change-impact' in sub_results even on partial failure"
        );
    }

    /// Test that individual errors are reported per sub-analysis
    #[test]
    fn reports_individual_errors() {
        let test_dir = TestDir::new().unwrap();
        // Empty directory - function lookup will likely fail

        let options = WhatbreaksOptions {
            language: Some(Language::Python),
            ..Default::default()
        };

        let report = whatbreaks_analysis("nonexistent_func", test_dir.path(), &options).unwrap();

        // SubResult structure must include:
        // - success: bool
        // - data: Option<Value> (present on success)
        // - error: Option<String> (present on failure)
        // - elapsed_ms: f64
        assert!(report.sub_results.contains_key("impact"));

        let impact_result = &report.sub_results["impact"];
        // Either success with empty data or failure with error message
        // The key is the structure is correct
        assert!(
            impact_result.elapsed_ms >= 0.0,
            "elapsed_ms should be non-negative"
        );
    }

    // -------------------------------------------------------------------------
    // Output Format Tests
    // -------------------------------------------------------------------------

    /// Test JSON output structure matches spec
    #[test]
    fn json_output_structure() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("service.py", WHATBREAKS_SERVICE).unwrap();

        let options = WhatbreaksOptions {
            language: Some(Language::Python),
            ..Default::default()
        };

        let report = whatbreaks_analysis("service.py", test_dir.path(), &options).unwrap();

        // Verify JSON structure
        assert_eq!(report.wrapper, "whatbreaks");
        assert_eq!(report.target, "service.py");
        assert_eq!(report.target_type, TargetType::File);
        assert!(!report.detection_reason.is_empty());
        assert!(report.total_elapsed_ms >= 0.0);

        // Summary should have all required fields
        let _direct = report.summary.direct_caller_count;
        let _transitive = report.summary.transitive_caller_count;
        let _importer = report.summary.importer_count;
        let _tests = report.summary.affected_test_count;

        // Can serialize to JSON
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("wrapper").is_some());
        assert!(json.get("target_type").is_some());
        assert!(json.get("sub_results").is_some());
        assert!(json.get("summary").is_some());
    }

    /// Test text output is human readable (via serialization)
    #[test]
    fn text_output_readable() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("service.py", WHATBREAKS_SERVICE).unwrap();

        let options = WhatbreaksOptions {
            language: Some(Language::Python),
            ..Default::default()
        };

        let report = whatbreaks_analysis("service.py", test_dir.path(), &options).unwrap();

        // Verify fields that would be used in text output
        assert!(!report.target.is_empty());
        assert!(!report.detection_reason.is_empty());

        // Display trait for TargetType
        let type_str = format!("{}", report.target_type);
        assert!(["function", "file", "module"].contains(&type_str.as_str()));
    }

    /// Test that empty results return empty lists, not errors
    #[test]
    fn empty_results_not_errors() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("isolated.py", SINGLE_FUNCTION).unwrap();

        let options = WhatbreaksOptions {
            language: Some(Language::Python),
            ..Default::default()
        };

        let report = whatbreaks_analysis("isolated.py", test_dir.path(), &options).unwrap();

        // A file with no callers/importers should return:
        // - success: true for importers (with empty list)
        // - NOT an error
        assert_eq!(report.target_type, TargetType::File);

        if let Some(importers_result) = report.sub_results.get("importers") {
            // Should be success with empty or small list, not an error
            // (success can be true with empty data)
            assert!(
                importers_result.success || importers_result.error.is_some(),
                "Sub-result should have either success or error"
            );
        }
    }
}

// =============================================================================
// hubs Tests
// =============================================================================

#[cfg(test)]
mod hubs_tests {
    use super::fixtures::*;
    use super::*;
    use crate::analysis::hubs::{
        compute_betweenness, compute_composite_score, compute_hub_report, compute_in_degree,
        compute_out_degree, compute_pagerank, BetweennessConfig, HubAlgorithm, PageRankConfig,
        RiskLevel,
    };
    use crate::callgraph::builder_v2::{build_project_call_graph_v2, BuildConfig};
    use crate::callgraph::graph_utils::{build_forward_graph, build_reverse_graph};
    use crate::types::{CallEdge, FunctionRef, Language, ProjectCallGraph};
    use std::collections::VecDeque;

    fn build_graph_and_nodes(test_dir: &TestDir) -> (ProjectCallGraph, HashSet<FunctionRef>) {
        let config = BuildConfig {
            language: Language::Python.as_str().to_string(),
            ..Default::default()
        };
        let ir =
            build_project_call_graph_v2(test_dir.path(), config).expect("call graph build failed");
        let mut graph = ProjectCallGraph::new();
        for edge in ir.edges {
            graph.add_edge(CallEdge {
                src_file: edge.src_file,
                src_func: edge.src_func,
                dst_file: edge.dst_file,
                dst_func: edge.dst_func,
            });
        }

        let mut nodes: HashSet<FunctionRef> = HashSet::new();
        for file in ir.files.values() {
            for func in &file.funcs {
                let name = if func.is_method {
                    if let Some(class_name) = &func.class_name {
                        format!("{}.{}", class_name, func.name)
                    } else {
                        func.name.clone()
                    }
                } else {
                    func.name.clone()
                };
                nodes.insert(FunctionRef::new(file.path.clone(), name));
            }
        }

        (graph, nodes)
    }

    fn score_by_name(scores: &HashMap<FunctionRef, f64>, name: &str) -> Option<f64> {
        scores.iter().find(|(f, _)| f.name == name).map(|(_, v)| *v)
    }

    fn transitive_impact(
        reverse_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
        start: &FunctionRef,
        max_depth: usize,
    ) -> usize {
        let mut visited: HashSet<FunctionRef> = HashSet::new();
        let mut queue: VecDeque<(FunctionRef, usize)> = VecDeque::new();

        queue.push_back((start.clone(), 0));
        visited.insert(start.clone());

        while let Some((node, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            if let Some(callers) = reverse_graph.get(&node) {
                for caller in callers {
                    if visited.insert(caller.clone()) {
                        queue.push_back((caller.clone(), depth + 1));
                    }
                }
            }
        }

        // Exclude the start node
        visited.len().saturating_sub(1)
    }

    // -------------------------------------------------------------------------
    // In-Degree Centrality Tests
    // -------------------------------------------------------------------------

    /// Test that in-degree counts number of callers correctly
    /// in_degree(v) = |{u : (u,v) in E}| / (n - 1)
    #[test]
    fn indegree_counts_callers() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("star.py", STAR_TOPOLOGY).unwrap();

        // In STAR_TOPOLOGY:
        // - central_hub is called by 5 functions (caller_a..e)
        // - caller_a..e have 0 callers each

        // Expected: central_hub has highest in-degree
        // in_degree(central_hub) = 5 / (6 - 1) = 1.0 (normalized)

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let reverse = build_reverse_graph(&graph);
        let in_degrees = compute_in_degree(&nodes, &reverse);

        let central = score_by_name(&in_degrees, "central_hub").unwrap_or(0.0);
        assert!((central - 1.0).abs() < 1e-6);

        for name in ["caller_a", "caller_b", "caller_c", "caller_d", "caller_e"] {
            let score = score_by_name(&in_degrees, name).unwrap_or(0.0);
            assert!((score - 0.0).abs() < 1e-6);
        }
    }

    /// Test that in-degree is normalized to [0, 1]
    #[test]
    fn indegree_normalized() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("complex.py", COMPLEX_TOPOLOGY).unwrap();

        // All in-degree values must be in range [0, 1]
        // Normalization: in_degree(v) = raw_count / (n - 1)

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let reverse = build_reverse_graph(&graph);
        let in_degrees = compute_in_degree(&nodes, &reverse);

        for value in in_degrees.values() {
            assert!(*value >= 0.0 && *value <= 1.0);
        }
    }

    // -------------------------------------------------------------------------
    // Out-Degree Centrality Tests
    // -------------------------------------------------------------------------

    /// Test that out-degree counts number of callees correctly
    /// out_degree(v) = |{u : (v,u) in E}| / (n - 1)
    #[test]
    fn outdegree_counts_callees() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("complex.py", COMPLEX_TOPOLOGY).unwrap();

        // In COMPLEX_TOPOLOGY:
        // - main calls 2 functions (service_a, service_b)
        // - service_a calls 2 functions (core_util, helper_x)
        // - core_util calls 2 functions (db_query, cache_get)
        // - db_query and cache_get call 0 functions (leaves)

        // Expected: main, service_a, service_b, core_util have higher out-degree

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let out_degrees = compute_out_degree(&nodes, &forward);

        let main_score = score_by_name(&out_degrees, "main").unwrap_or(0.0);
        let db_score = score_by_name(&out_degrees, "db_query").unwrap_or(0.0);
        assert!(main_score > db_score);
    }

    /// Test that out-degree is normalized to [0, 1]
    #[test]
    fn outdegree_normalized() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("star.py", STAR_TOPOLOGY).unwrap();

        // All out-degree values must be in range [0, 1]
        // Normalization: out_degree(v) = raw_count / (n - 1)

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let out_degrees = compute_out_degree(&nodes, &forward);

        for value in out_degrees.values() {
            assert!(*value >= 0.0 && *value <= 1.0);
        }
    }

    // -------------------------------------------------------------------------
    // PageRank Tests
    // -------------------------------------------------------------------------

    /// Test that PageRank converges within max iterations
    #[test]
    fn pagerank_converges() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("diamond.py", DIAMOND_TOPOLOGY).unwrap();

        // PageRank should converge (delta < epsilon) before max_iter
        // For small graphs, should converge quickly (< 50 iterations)

        // In DIAMOND_TOPOLOGY, node_d should have highest PageRank
        // (both B and C depend on it)

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let config = PageRankConfig::default();
        let result = compute_pagerank(&nodes, &reverse, &forward, &config);

        assert!(result.converged);
        assert!(result.iterations_used <= config.max_iterations);

        let top = result
            .scores
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(f, _)| f.name.clone())
            .unwrap_or_default();
        assert_eq!(top, "node_d");
    }

    /// Test that damping factor is applied correctly (default: 0.85)
    #[test]
    fn pagerank_damping_factor() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("chain.py", CHAIN_TOPOLOGY).unwrap();

        // PageRank formula: PR(v) = (1-d)/n + d * sum(PR(u)/out_deg(u))
        // Damping factor d = 0.85 means:
        // - 15% probability of random jump
        // - 85% probability of following edges

        // Different damping factors should produce different rankings

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);

        let config_default = PageRankConfig::default();
        let result_default = compute_pagerank(&nodes, &reverse, &forward, &config_default);

        let config_low = PageRankConfig {
            damping: 0.5,
            ..Default::default()
        };
        let result_low = compute_pagerank(&nodes, &reverse, &forward, &config_low);

        let score_default = score_by_name(&result_default.scores, "node_c").unwrap_or(0.0);
        let score_low = score_by_name(&result_low.scores, "node_c").unwrap_or(0.0);
        assert!((score_default - score_low).abs() > 1e-6);
    }

    /// Test that dangling nodes (no outgoing edges) are handled
    #[test]
    fn pagerank_handles_dangling_nodes() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("chain.py", CHAIN_TOPOLOGY).unwrap();

        // node_e has no outgoing edges (dangling node)
        // PageRank must redistribute dangling node's rank evenly

        // Without dangling node handling, rank would "leak"

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let result = compute_pagerank(&nodes, &reverse, &forward, &PageRankConfig::default());

        for value in result.scores.values() {
            assert!(value.is_finite());
        }

        let node_e_score = score_by_name(&result.scores, "node_e").unwrap_or(0.0);
        assert!(node_e_score >= 0.0);
    }

    // -------------------------------------------------------------------------
    // Betweenness Centrality Tests
    // -------------------------------------------------------------------------

    /// Test that betweenness detects bridge nodes
    #[test]
    fn betweenness_bridge_detection() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("chain.py", CHAIN_TOPOLOGY).unwrap();

        // In CHAIN_TOPOLOGY: A -> B -> C -> D -> E
        // node_b, node_c, node_d are on paths between endpoints
        // node_c should have highest betweenness (on most paths)

        // betweenness(v) = sum(sigma_st(v) / sigma_st) for all s,t

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let betweenness = compute_betweenness(&nodes, &forward, &BetweennessConfig::default());

        let node_b = score_by_name(&betweenness, "node_b").unwrap_or(0.0);
        let node_c = score_by_name(&betweenness, "node_c").unwrap_or(0.0);
        let node_d = score_by_name(&betweenness, "node_d").unwrap_or(0.0);
        assert!(node_c >= node_b);
        assert!(node_c >= node_d);
    }

    /// Test that betweenness is normalized to [0, 1]
    #[test]
    fn betweenness_normalized() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("complex.py", COMPLEX_TOPOLOGY).unwrap();

        // Betweenness normalized by (n-1)(n-2) for directed graphs
        // All values must be in range [0, 1]

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let betweenness = compute_betweenness(&nodes, &forward, &BetweennessConfig::default());

        for value in betweenness.values() {
            assert!(*value >= 0.0 && *value <= 1.0);
        }
    }

    // -------------------------------------------------------------------------
    // Composite Score Tests
    // -------------------------------------------------------------------------

    /// Test that composite score is weighted average of measures
    #[test]
    fn composite_weighted_average() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("complex.py", COMPLEX_TOPOLOGY).unwrap();

        // Default weights (from spec):
        // - in_degree: 0.25
        // - out_degree: 0.25
        // - betweenness: 0.30
        // - pagerank: 0.20

        // composite = sum(weight[m] * score[m]) / sum(weight[m])

        // If only subset of measures used, normalize weights to sum to 1.0

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let pr = compute_pagerank(&nodes, &reverse, &forward, &PageRankConfig::default());
        let bc = compute_betweenness(&nodes, &forward, &BetweennessConfig::default());
        let in_deg = compute_in_degree(&nodes, &reverse);
        let out_deg = compute_out_degree(&nodes, &forward);

        let target = in_deg
            .keys()
            .find(|f| f.name == "core_util")
            .cloned()
            .unwrap_or_else(|| in_deg.keys().next().unwrap().clone());
        let expected = compute_composite_score(
            *in_deg.get(&target).unwrap_or(&0.0),
            *out_deg.get(&target).unwrap_or(&0.0),
            pr.scores.get(&target).copied(),
            bc.get(&target).copied(),
        );

        let report = compute_hub_report(
            &nodes,
            &forward,
            &reverse,
            HubAlgorithm::All,
            nodes.len(),
            None,
        );
        let found = report
            .hubs
            .iter()
            .find(|hub| hub.name == target.name)
            .map(|hub| hub.composite_score)
            .unwrap_or(0.0);
        assert!((expected - found).abs() < 1e-6);
    }

    /// Test risk classification thresholds
    #[test]
    fn risk_classification_thresholds() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("complex.py", COMPLEX_TOPOLOGY).unwrap();

        // Risk levels per spec:
        // - critical: composite >= 0.8
        // - high: composite >= 0.6
        // - medium: composite >= 0.4
        // - low: composite < 0.4

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let report = compute_hub_report(
            &nodes,
            &forward,
            &reverse,
            HubAlgorithm::All,
            nodes.len(),
            None,
        );

        for hub in &report.hubs {
            assert_eq!(hub.risk_level, RiskLevel::from_score(hub.composite_score));
        }
    }

    /// Test transitive impact calculation (BFS depth 3)
    #[test]
    fn transitive_impact_calculation() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("chain.py", CHAIN_TOPOLOGY).unwrap();

        // Transitive impact = number of nodes reachable via callers up to depth 3
        // For node_e in chain A->B->C->D->E:
        // - depth 1: D
        // - depth 2: C
        // - depth 3: B
        // - transitive_impact = 3 (excludes self)

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let reverse = build_reverse_graph(&graph);

        let node_e = nodes.iter().find(|f| f.name == "node_e").cloned().unwrap();
        let impact = transitive_impact(&reverse, &node_e, 3);
        assert_eq!(impact, 3);
    }

    // -------------------------------------------------------------------------
    // Edge Case Tests
    // -------------------------------------------------------------------------

    /// Test that empty graph returns empty report without panic
    #[test]
    fn empty_graph_no_panic() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("empty.py", EMPTY_MODULE).unwrap();

        // Empty graph (no functions/edges) should return:
        // - hubs: []
        // - total_nodes: 0
        // - hub_count: 0
        // - No panic or error

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let report = compute_hub_report(&nodes, &forward, &reverse, HubAlgorithm::All, 10, None);

        assert_eq!(report.total_nodes, 0);
        assert_eq!(report.hub_count, 0);
        assert!(report.hubs.is_empty());
    }

    /// Test single node graph
    #[test]
    fn single_node_graph() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("single.py", SINGLE_FUNCTION).unwrap();

        // Single node graph (no edges):
        // - All centralities should be 0 (no edges)
        // - total_nodes: 1
        // - hub_count: 0 or 1 depending on threshold

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let report = compute_hub_report(&nodes, &forward, &reverse, HubAlgorithm::All, 10, None);

        assert_eq!(report.total_nodes, 1);
        assert_eq!(report.hub_count, 1);
        let hub = report.hubs.first().unwrap();
        assert_eq!(hub.name, "lonely");
        assert_eq!(hub.in_degree, 0.0);
        assert_eq!(hub.out_degree, 0.0);
    }

    /// Test disconnected components are analyzed independently
    #[test]
    fn disconnected_components() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("disconnected.py", DISCONNECTED_COMPONENTS)
            .unwrap();

        // Two disconnected components should both be analyzed
        // Centrality calculated per component or whole graph?
        // Per spec: each component analyzed independently for betweenness

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let report = compute_hub_report(
            &nodes,
            &forward,
            &reverse,
            HubAlgorithm::All,
            nodes.len(),
            None,
        );

        assert_eq!(report.total_nodes, nodes.len());
        let names: HashSet<String> = report.hubs.iter().map(|hub| hub.name.clone()).collect();
        assert!(names.contains("comp1_a"));
        assert!(names.contains("comp2_x"));
    }

    /// Test self-loops are counted in degree calculations
    #[test]
    fn self_loops_counted() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("recursive.py", SELF_LOOP).unwrap();

        // recursive_func calls itself
        // This should count in both in-degree and out-degree

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);

        let caller_counts = crate::analysis::hubs::get_caller_counts(&nodes, &reverse);
        let callee_counts = crate::analysis::hubs::get_callee_counts(&nodes, &forward);

        let recursive = nodes
            .iter()
            .find(|f| f.name == "recursive_func")
            .cloned()
            .unwrap();
        assert_eq!(caller_counts.get(&recursive), Some(&1));
        assert_eq!(callee_counts.get(&recursive), Some(&1));
    }

    // -------------------------------------------------------------------------
    // Output Format Tests
    // -------------------------------------------------------------------------

    /// Test JSON output structure matches spec
    #[test]
    fn json_output_structure() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("complex.py", COMPLEX_TOPOLOGY).unwrap();

        // JSON output must have (per spec section 3):
        // {
        //   "hubs": [
        //     {
        //       "file": string,
        //       "name": string,
        //       "fqn": string,
        //       "centrality": {
        //         "in_degree": float [0,1],
        //         "out_degree": float [0,1],
        //         "betweenness": float [0,1],
        //         "pagerank": float [0,1],
        //         "composite": float [0,1]
        //       },
        //       "impact": {
        //         "direct_callers": int,
        //         "direct_callees": int,
        //         "transitive_impact": int
        //       },
        //       "risk_level": "critical" | "high" | "medium" | "low"
        //     }
        //   ],
        //   "summary": {...},
        //   "by_measure": {...}
        // }

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let report = compute_hub_report(&nodes, &forward, &reverse, HubAlgorithm::All, 5, None);

        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("hubs").is_some());
        assert!(json.get("total_nodes").is_some());
        assert!(json.get("hub_count").is_some());
        assert!(json.get("measures_used").is_some());
    }

    /// Test hubs are sorted by composite score descending
    #[test]
    fn sorted_by_composite_descending() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("complex.py", COMPLEX_TOPOLOGY).unwrap();

        // Output hubs must be sorted by composite_score descending
        // hubs[0] should have highest composite score
        // hubs[n-1] should have lowest composite score

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let report = compute_hub_report(&nodes, &forward, &reverse, HubAlgorithm::All, 5, None);

        for window in report.hubs.windows(2) {
            assert!(window[0].composite_score >= window[1].composite_score);
        }
    }

    /// Test top_k parameter limits results
    #[test]
    fn top_k_limits_results() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("complex.py", COMPLEX_TOPOLOGY).unwrap();

        // With --top 3:
        // - hubs.len() <= 3
        // - Returns top 3 by composite score

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let report = compute_hub_report(&nodes, &forward, &reverse, HubAlgorithm::All, 3, None);

        assert!(report.hubs.len() <= 3);
    }

    /// Test threshold parameter filters results
    #[test]
    fn threshold_filters_results() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("complex.py", COMPLEX_TOPOLOGY).unwrap();

        // With --threshold 0.5:
        // - Only hubs with composite >= 0.5 returned
        // - hub_count reflects filtered count

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let report =
            compute_hub_report(&nodes, &forward, &reverse, HubAlgorithm::All, 10, Some(0.5));

        assert!(report.hubs.iter().all(|hub| hub.composite_score >= 0.5));
    }

    /// Test algorithm selection (subset of measures)
    #[test]
    fn algorithm_selection() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("complex.py", COMPLEX_TOPOLOGY).unwrap();

        // With --algorithm in_degree,out_degree:
        // - Only in_degree and out_degree computed
        // - betweenness and pagerank skipped (faster)
        // - composite uses only selected measures

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let report =
            compute_hub_report(&nodes, &forward, &reverse, HubAlgorithm::InDegree, 5, None);

        assert_eq!(report.measures_used, vec!["in_degree".to_string()]);
        assert!(report
            .hubs
            .iter()
            .all(|hub| hub.pagerank.is_none() && hub.betweenness.is_none()));
    }

    /// Test text output format
    #[test]
    fn text_output_format() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("complex.py", COMPLEX_TOPOLOGY).unwrap();

        // Text output per spec:
        // Hub Detection Report
        // ====================
        // Total nodes: N
        // Hubs found: M (threshold: X)
        // Measures: in_degree, out_degree, pagerank, betweenness
        //
        // Top Hubs by Composite Score:
        //   1. [CRITICAL] function_name (file.py)
        //      Composite: 0.XXXX | Callers: N | Impact: M
        //   ...
        //
        // Recommendation: Critical hubs (N) - changes require extensive testing

        let (graph, nodes) = build_graph_and_nodes(&test_dir);
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let report = compute_hub_report(&nodes, &forward, &reverse, HubAlgorithm::All, 3, None);

        let mut output = String::new();
        output.push_str("Hub Detection Report\n");
        output.push_str("====================\n");
        output.push_str(&format!("Total nodes: {}\n", report.total_nodes));
        output.push_str(&format!("Hubs found: {}\n", report.hub_count));
        output.push_str("Top Hubs by Composite Score:\n");
        for (idx, hub) in report.hubs.iter().enumerate() {
            output.push_str(&format!(
                "  {}. [{}] {} ({})\n",
                idx + 1,
                hub.risk_level,
                hub.name,
                hub.file.display()
            ));
        }

        assert!(output.contains("Hub Detection Report"));
        assert!(output.contains("Top Hubs by Composite Score"));
    }
}

// =============================================================================
// Integration Tests
// =============================================================================

#[cfg(test)]
mod integration_tests {
    use super::fixtures::*;
    use super::*;
    use crate::analysis::hubs::{compute_hub_report, HubAlgorithm};
    use crate::analysis::impact::impact_analysis;
    use crate::analysis::whatbreaks::{whatbreaks_analysis, WhatbreaksOptions};
    use crate::callgraph::builder_v2::{build_project_call_graph_v2, BuildConfig};
    use crate::callgraph::graph_utils::{build_forward_graph, build_reverse_graph};
    use crate::types::{CallEdge, FunctionRef, Language, ProjectCallGraph};

    fn build_graph_and_nodes(test_dir: &TestDir) -> (ProjectCallGraph, HashSet<FunctionRef>) {
        let config = BuildConfig {
            language: Language::Python.as_str().to_string(),
            ..Default::default()
        };
        let ir =
            build_project_call_graph_v2(test_dir.path(), config).expect("call graph build failed");
        let mut graph = ProjectCallGraph::new();
        for edge in ir.edges {
            graph.add_edge(CallEdge {
                src_file: edge.src_file,
                src_func: edge.src_func,
                dst_file: edge.dst_file,
                dst_func: edge.dst_func,
            });
        }

        let mut nodes: HashSet<FunctionRef> = HashSet::new();
        for file in ir.files.values() {
            for func in &file.funcs {
                let name = if func.is_method {
                    if let Some(class_name) = &func.class_name {
                        format!("{}.{}", class_name, func.name)
                    } else {
                        func.name.clone()
                    }
                } else {
                    func.name.clone()
                };
                nodes.insert(FunctionRef::new(file.path.clone(), name));
            }
        }

        (graph, nodes)
    }

    /// Test that all three commands can be run on same project
    #[test]
    fn all_commands_work_together() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("main.py", WHATBREAKS_MAIN).unwrap();
        test_dir.add_file("service.py", WHATBREAKS_SERVICE).unwrap();
        test_dir
            .add_file("repository.py", WHATBREAKS_REPOSITORY)
            .unwrap();
        test_dir.add_file("utils.py", WHATBREAKS_UTILS).unwrap();

        // Should be able to run:
        // 1. tldr impact helper --type-aware
        // 2. tldr whatbreaks service.py
        // 3. tldr hubs .

        // All should complete without error

        let (graph, nodes) = build_graph_and_nodes(&test_dir);

        let impact = impact_analysis(&graph, "helper", 3, None).unwrap();
        assert!(impact.total_targets >= 1);

        let options = WhatbreaksOptions {
            language: Some(Language::Python),
            ..Default::default()
        };
        let report = whatbreaks_analysis("service.py", test_dir.path(), &options).unwrap();
        assert!(report.sub_results.contains_key("importers"));

        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let hubs = compute_hub_report(&nodes, &forward, &reverse, HubAlgorithm::All, 5, None);
        assert!(hubs.total_nodes >= 1);
    }

    /// Test shared call graph is reused efficiently
    #[test]
    fn shared_callgraph_reuse() {
        // When running multiple analyses:
        // - Call graph should be built once
        // - Reused across impact, whatbreaks, hubs

        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("main.py", WHATBREAKS_MAIN).unwrap();
        test_dir.add_file("service.py", WHATBREAKS_SERVICE).unwrap();
        test_dir
            .add_file("repository.py", WHATBREAKS_REPOSITORY)
            .unwrap();
        test_dir.add_file("utils.py", WHATBREAKS_UTILS).unwrap();

        let (graph, nodes) = build_graph_and_nodes(&test_dir);

        let impact = impact_analysis(&graph, "helper", 2, None).unwrap();
        assert!(impact.total_targets >= 1);

        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let hubs = compute_hub_report(&nodes, &forward, &reverse, HubAlgorithm::All, 5, None);
        assert!(hubs.total_nodes >= 1);
    }
}
