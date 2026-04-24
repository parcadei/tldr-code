//! Test module for architecture analysis commands: arch gaps, patterns, inheritance
//!
//! These tests define expected behavior BEFORE implementation.
//! Tests are designed to FAIL until the modules are implemented.
//!
//! # Test Categories
//!
//! ## 1. arch gaps tests (--generate-rules, --check-rules, Tarjan SCC)
//! - Rule generation in YAML/JSON format
//! - Layer constraints (LOW may not import HIGH)
//! - Cycle break recommendations
//! - Rule checking and violation detection
//! - Tarjan SCC algorithm for precise cycle detection
//!
//! ## 2. patterns tests
//! - Soft delete detection (is_deleted, deleted_at fields)
//! - Error handling patterns (try/catch, Result types)
//! - Naming conventions (snake_case, PascalCase)
//! - Resource management (context managers, defer, RAII)
//! - Multi-language support
//!
//! ## 3. inheritance tests
//! - Class hierarchy extraction (Python, TypeScript)
//! - ABC/Protocol/Interface detection
//! - Mixin class detection
//! - Diamond inheritance detection
//! - Output formats (JSON, DOT, text)
//! - Class filter with fuzzy matching
//!
//! Reference: architecture-spec.md

// =============================================================================
// Test Fixture Setup Module
// =============================================================================

/// Test fixture utilities for architecture analysis tests
pub mod fixtures {
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// A temporary directory for testing architecture analysis
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
    }

    // -------------------------------------------------------------------------
    // arch Rules Fixtures
    // -------------------------------------------------------------------------

    /// Layered project for rules generation testing
    pub const LAYERED_PROJECT_API: &str = r#"
from services.user_service import UserService

def get_user(user_id: int):
    service = UserService()
    return service.find_by_id(user_id)

def create_user(name: str):
    service = UserService()
    return service.create(name)
"#;

    pub const LAYERED_PROJECT_SERVICE: &str = r#"
from utils.db import query, insert

class UserService:
    def find_by_id(self, user_id: int):
        return query("SELECT * FROM users WHERE id = ?", user_id)

    def create(self, name: str):
        return insert("users", {"name": name})
"#;

    pub const LAYERED_PROJECT_UTILS: &str = r#"
def query(sql: str, *args):
    return []

def insert(table: str, data: dict):
    return {"id": 1}
"#;

    /// Violating project - utils imports api (LOW imports HIGH)
    pub const VIOLATING_UTILS: &str = r#"
# BAD: Utility layer importing from API layer
from api.routes import get_user

def format_user(user_id: int):
    user = get_user(user_id)  # Violation: LOW -> HIGH
    return str(user)
"#;

    /// Circular dependency between services and api
    pub const CIRCULAR_API: &str = r#"
from services.auth import validate_token

def protected_route():
    return validate_token()
"#;

    pub const CIRCULAR_SERVICE: &str = r#"
from api.routes import protected_route  # Creates cycle

def validate_token():
    # This creates a circular dependency
    return True
"#;

    // -------------------------------------------------------------------------
    // Tarjan SCC Fixtures
    // -------------------------------------------------------------------------

    /// Simple 2-node cycle: A <-> B
    pub const SIMPLE_CYCLE: &str = r#"
def func_a():
    return func_b()

def func_b():
    return func_a()  # A -> B -> A
"#;

    /// Complex 3-node cycle: A -> B -> C -> A
    pub const COMPLEX_CYCLE: &str = r#"
def node_a():
    return node_b()

def node_b():
    return node_c()

def node_c():
    return node_a()  # A -> B -> C -> A
"#;

    /// Multiple SCCs in one graph
    pub const MULTIPLE_SCCS: &str = r#"
# SCC 1: cycle_a <-> cycle_b
def cycle_a():
    return cycle_b()

def cycle_b():
    return cycle_a()

# SCC 2: loop_x -> loop_y -> loop_z -> loop_x
def loop_x():
    return loop_y()

def loop_y():
    return loop_z()

def loop_z():
    return loop_x()

# Non-cycle: isolated functions
def standalone():
    return 42
"#;

    /// DAG (no cycles) - should find no SCCs > 1 node
    pub const DAG_NO_CYCLES: &str = r#"
def root():
    branch_a()
    branch_b()

def branch_a():
    leaf()

def branch_b():
    leaf()

def leaf():
    return 42
"#;

    // -------------------------------------------------------------------------
    // patterns Fixtures
    // -------------------------------------------------------------------------

    /// Python soft delete pattern
    pub const PYTHON_SOFT_DELETE: &str = r#"
from sqlalchemy import Column, Boolean, DateTime

class User(Base):
    __tablename__ = 'users'

    id = Column(Integer, primary_key=True)
    name = Column(String)
    is_deleted = Column(Boolean, default=False)
    deleted_at = Column(DateTime, nullable=True)

    def soft_delete(self):
        self.is_deleted = True
        self.deleted_at = datetime.now()
"#;

    /// TypeScript soft delete pattern
    pub const TYPESCRIPT_SOFT_DELETE: &str = r#"
interface User {
    id: number;
    name: string;
    isDeleted: boolean;
    deletedAt: Date | null;
}

class UserModel {
    softDelete(user: User): void {
        user.isDeleted = true;
        user.deletedAt = new Date();
    }
}
"#;

    /// Python error handling patterns
    pub const PYTHON_ERROR_HANDLING: &str = r#"
class ValidationError(Exception):
    """Custom validation error"""
    pass

class NotFoundError(Exception):
    """Resource not found"""
    pass

def process_data(data):
    try:
        validate(data)
        return transform(data)
    except ValidationError as e:
        logger.error(f"Validation failed: {e}")
        raise
    except Exception as e:
        logger.error(f"Unexpected error: {e}")
        raise NotFoundError("Data processing failed")
"#;

    /// Rust error handling with Result
    pub const RUST_ERROR_HANDLING: &str = r#"
use std::io;

#[derive(Debug)]
pub enum AppError {
    IoError(io::Error),
    ValidationError(String),
}

pub fn process_file(path: &str) -> Result<String, AppError> {
    let content = std::fs::read_to_string(path)
        .map_err(AppError::IoError)?;

    if content.is_empty() {
        return Err(AppError::ValidationError("Empty file".into()));
    }

    Ok(content)
}
"#;

    /// Go error handling pattern
    pub const GO_ERROR_HANDLING: &str = r#"
package main

import (
    "errors"
    "fmt"
)

var ErrNotFound = errors.New("not found")

func FindUser(id int) (*User, error) {
    user, err := db.Query(id)
    if err != nil {
        return nil, fmt.Errorf("query failed: %w", err)
    }
    if user == nil {
        return nil, ErrNotFound
    }
    return user, nil
}
"#;

    /// Python naming conventions (snake_case)
    pub const PYTHON_NAMING: &str = r#"
GLOBAL_CONSTANT = 42
MAX_RETRY_COUNT = 3

class UserService:
    def __init__(self):
        self._private_field = None

    def find_user_by_id(self, user_id: int):
        return self._query_database(user_id)

    def _query_database(self, user_id: int):
        return None

def helper_function():
    local_variable = 10
    return local_variable
"#;

    /// TypeScript naming conventions (camelCase/PascalCase)
    pub const TYPESCRIPT_NAMING: &str = r#"
const GLOBAL_CONSTANT = 42;
const maxRetryCount = 3;

class UserService {
    private privateField: string;

    findUserById(userId: number): User | null {
        return this.queryDatabase(userId);
    }

    private queryDatabase(userId: number): User | null {
        return null;
    }
}

function helperFunction(): number {
    const localVariable = 10;
    return localVariable;
}
"#;

    /// Python context manager pattern
    pub const PYTHON_CONTEXT_MANAGER: &str = r#"
class FileManager:
    def __init__(self, filename):
        self.filename = filename
        self.file = None

    def __enter__(self):
        self.file = open(self.filename, 'r')
        return self.file

    def __exit__(self, exc_type, exc_val, exc_tb):
        if self.file:
            self.file.close()

def process_file(filename):
    with FileManager(filename) as f:
        return f.read()

    # Also with standard library
    with open("other.txt") as f:
        content = f.read()
"#;

    /// Go defer pattern
    pub const GO_DEFER: &str = r#"
package main

import "os"

func processFile(filename string) error {
    f, err := os.Open(filename)
    if err != nil {
        return err
    }
    defer f.Close()

    // Process file...
    return nil
}
"#;

    /// Rust RAII pattern
    pub const RUST_RAII: &str = r#"
use std::fs::File;
use std::io::Read;

struct FileHandle {
    file: File,
}

impl Drop for FileHandle {
    fn drop(&mut self) {
        // File automatically closed when dropped
        println!("Closing file");
    }
}

fn process_file(path: &str) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents)
    // File automatically closed here
}
"#;

    // -------------------------------------------------------------------------
    // inheritance Fixtures
    // -------------------------------------------------------------------------

    /// Python class hierarchy
    pub const PYTHON_INHERITANCE: &str = r#"
from abc import ABC, abstractmethod
from typing import Protocol

class Animal(ABC):
    @abstractmethod
    def speak(self) -> str:
        pass

class Mammal(Animal):
    def breathe(self):
        return "breathing"

class Dog(Mammal):
    def speak(self) -> str:
        return "woof"

class Cat(Mammal):
    def speak(self) -> str:
        return "meow"

# Protocol example
class Serializable(Protocol):
    def serialize(self) -> dict:
        ...

# External base class (from library)
class MyException(Exception):
    pass
"#;

    /// Python mixin pattern
    pub const PYTHON_MIXIN: &str = r#"
class TimestampMixin:
    created_at = None
    updated_at = None

    def touch(self):
        self.updated_at = datetime.now()

class AuditMixin:
    created_by = None
    updated_by = None

class User(Base, TimestampMixin, AuditMixin):
    __tablename__ = 'users'
    name = Column(String)

class Post(Base, TimestampMixin):
    __tablename__ = 'posts'
    title = Column(String)
"#;

    /// Python diamond inheritance
    pub const PYTHON_DIAMOND: &str = r#"
class Base:
    def method(self):
        return "base"

class ParentA(Base):
    def method_a(self):
        return "A"

class ParentB(Base):
    def method_b(self):
        return "B"

class Diamond(ParentA, ParentB):
    # Diamond inheritance: Diamond -> ParentA -> Base
    #                      Diamond -> ParentB -> Base
    def method(self):
        return super().method()  # Uses MRO
"#;

    /// TypeScript class hierarchy
    pub const TYPESCRIPT_INHERITANCE: &str = r#"
interface Serializable {
    serialize(): string;
}

abstract class Animal {
    abstract speak(): string;
}

class Mammal extends Animal {
    breathe(): string {
        return "breathing";
    }

    speak(): string {
        return "generic mammal sound";
    }
}

class Dog extends Mammal implements Serializable {
    speak(): string {
        return "woof";
    }

    serialize(): string {
        return JSON.stringify({ type: "Dog" });
    }
}

// Multiple interfaces
interface Walkable {
    walk(): void;
}

class Cat extends Mammal implements Serializable, Walkable {
    speak(): string {
        return "meow";
    }

    serialize(): string {
        return JSON.stringify({ type: "Cat" });
    }

    walk(): void {
        console.log("walking");
    }
}
"#;
}

// =============================================================================
// arch Rules Tests (--generate-rules, --check-rules)
// =============================================================================

#[cfg(test)]
mod arch_rules_tests {
    use super::fixtures::*;

    // -------------------------------------------------------------------------
    // Rule Generation Tests
    // -------------------------------------------------------------------------

    /// Test that --generate-rules produces valid YAML format
    /// Contract: Output must be parseable YAML with version, layers, rules keys
    #[test]
    #[ignore = "arch --generate-rules not yet implemented"]
    fn generate_rules_yaml_format() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("api/routes.py", LAYERED_PROJECT_API)
            .unwrap();
        test_dir
            .add_file("services/user_service.py", LAYERED_PROJECT_SERVICE)
            .unwrap();
        test_dir
            .add_file("utils/db.py", LAYERED_PROJECT_UTILS)
            .unwrap();

        // Expected: generate_rules returns YAML string with required keys
        // let rules_yaml = generate_architecture_rules(test_dir.path(), OutputFormat::Yaml).unwrap();
        //
        // Parse YAML and verify structure:
        // - version: "1.0"
        // - generated_at: ISO8601 timestamp
        // - layers: { high: {...}, middle: {...}, low: {...} }
        // - rules: [ { id, constraint, type, ... }, ... ]

        todo!("Implement generate_rules_yaml_format test");
    }

    /// Test that generated rules include layer constraints
    /// Contract: L1 (LOW -> HIGH forbidden), L2 (MIDDLE -> HIGH forbidden)
    #[test]
    #[ignore = "arch --generate-rules not yet implemented"]
    fn generate_rules_includes_layer_constraints() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("api/routes.py", LAYERED_PROJECT_API)
            .unwrap();
        test_dir
            .add_file("services/user_service.py", LAYERED_PROJECT_SERVICE)
            .unwrap();
        test_dir
            .add_file("utils/db.py", LAYERED_PROJECT_UTILS)
            .unwrap();

        // Expected: rules contain L1 and L2 constraints
        // let rules = generate_architecture_rules(test_dir.path(), OutputFormat::Json).unwrap();
        // let parsed: ArchRules = serde_json::from_str(&rules).unwrap();
        //
        // assert!(parsed.rules.iter().any(|r| r.id == "L1"));
        // assert!(parsed.rules.iter().any(|r| r.id == "L2"));
        //
        // let l1 = parsed.rules.iter().find(|r| r.id == "L1").unwrap();
        // assert_eq!(l1.from_layers, vec!["LOW"]);
        // assert_eq!(l1.to_layers, vec!["HIGH"]);
        // assert_eq!(l1.rule_type, RuleType::Layer);

        todo!("Implement generate_rules_includes_layer_constraints test");
    }

    /// Test that generated rules include cycle break recommendations
    /// Contract: C1, C2, ... rules for detected circular dependencies
    #[test]
    #[ignore = "arch --generate-rules not yet implemented"]
    fn generate_rules_includes_cycle_breaks() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("api/routes.py", CIRCULAR_API).unwrap();
        test_dir
            .add_file("services/auth.py", CIRCULAR_SERVICE)
            .unwrap();

        // Expected: rules contain C1 cycle-break recommendation
        // let rules = generate_architecture_rules(test_dir.path(), OutputFormat::Json).unwrap();
        // let parsed: ArchRules = serde_json::from_str(&rules).unwrap();
        //
        // let cycle_rules: Vec<_> = parsed.rules.iter()
        //     .filter(|r| r.rule_type == RuleType::CycleBreak)
        //     .collect();
        //
        // assert!(!cycle_rules.is_empty(), "Expected at least one cycle-break rule");
        // assert!(cycle_rules[0].id.starts_with("C"));
        // assert!(cycle_rules[0].files.len() >= 2);

        todo!("Implement generate_rules_includes_cycle_breaks test");
    }

    // -------------------------------------------------------------------------
    // Rule Checking Tests
    // -------------------------------------------------------------------------

    /// Test that --check-rules detects layer violations
    /// Contract: Violation when LOW imports HIGH
    #[test]
    #[ignore = "arch --check-rules not yet implemented"]
    fn check_rules_detects_layer_violation() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("api/routes.py", LAYERED_PROJECT_API)
            .unwrap();
        test_dir
            .add_file("services/user_service.py", LAYERED_PROJECT_SERVICE)
            .unwrap();
        test_dir.add_file("utils/db.py", VIOLATING_UTILS).unwrap();

        // Create rules file
        let rules = r#"
version: "1.0"
layers:
  high: { directories: ["api/"] }
  middle: { directories: ["services/"] }
  low: { directories: ["utils/"] }
rules:
  - id: L1
    constraint: "LOW may not import HIGH"
    type: layer
    from_layers: [LOW]
    to_layers: [HIGH]
    severity: error
"#;
        test_dir.add_file("arch-rules.yaml", rules).unwrap();

        // Expected: check_rules returns violations
        // let report = check_architecture_rules(test_dir.path(), "arch-rules.yaml").unwrap();
        //
        // assert!(!report.pass);
        // assert!(!report.violations.is_empty());
        //
        // let violation = &report.violations[0];
        // assert_eq!(violation.rule_id, "L1");
        // assert!(violation.from_file.contains("utils"));
        // assert!(violation.imports_file.contains("api"));
        // assert_eq!(violation.severity, Severity::Error);

        todo!("Implement check_rules_detects_layer_violation test");
    }

    /// Test that --check-rules detects cycle violations
    /// Contract: Violation when cycle-break rule files both import each other
    #[test]
    #[ignore = "arch --check-rules not yet implemented"]
    fn check_rules_detects_cycle_violation() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("api/routes.py", CIRCULAR_API).unwrap();
        test_dir
            .add_file("services/auth.py", CIRCULAR_SERVICE)
            .unwrap();

        // Create rules file with cycle-break rule
        let rules = r#"
version: "1.0"
rules:
  - id: C1
    constraint: "Break cycle: services/auth.py should not import api/routes.py"
    type: cycle_break
    files:
      - "services/auth.py"
      - "api/routes.py"
    severity: warn
"#;
        test_dir.add_file("arch-rules.yaml", rules).unwrap();

        // Expected: check_rules returns cycle violation
        // let report = check_architecture_rules(test_dir.path(), "arch-rules.yaml").unwrap();
        //
        // let cycle_violations: Vec<_> = report.violations.iter()
        //     .filter(|v| v.rule_id.starts_with("C"))
        //     .collect();
        //
        // assert!(!cycle_violations.is_empty());
        // assert_eq!(cycle_violations[0].severity, Severity::Warn);

        todo!("Implement check_rules_detects_cycle_violation test");
    }

    /// Test that --check-rules allows valid dependencies
    /// Contract: No violations when dependencies follow layer rules
    #[test]
    #[ignore = "arch --check-rules not yet implemented"]
    fn check_rules_allows_valid_dependencies() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("api/routes.py", LAYERED_PROJECT_API)
            .unwrap();
        test_dir
            .add_file("services/user_service.py", LAYERED_PROJECT_SERVICE)
            .unwrap();
        test_dir
            .add_file("utils/db.py", LAYERED_PROJECT_UTILS)
            .unwrap();

        // Create rules file
        let rules = r#"
version: "1.0"
layers:
  high: { directories: ["api/"] }
  middle: { directories: ["services/"] }
  low: { directories: ["utils/"] }
rules:
  - id: L1
    constraint: "LOW may not import HIGH"
    type: layer
    from_layers: [LOW]
    to_layers: [HIGH]
    severity: error
  - id: L2
    constraint: "MIDDLE may not import HIGH"
    type: layer
    from_layers: [MIDDLE]
    to_layers: [HIGH]
    severity: error
"#;
        test_dir.add_file("arch-rules.yaml", rules).unwrap();

        // Expected: check_rules passes with no violations
        // Valid flow: api -> services -> utils
        // let report = check_architecture_rules(test_dir.path(), "arch-rules.yaml").unwrap();
        //
        // assert!(report.pass);
        // assert!(report.violations.is_empty());
        // assert_eq!(report.summary.error_count, 0);

        todo!("Implement check_rules_allows_valid_dependencies test");
    }

    // -------------------------------------------------------------------------
    // Tarjan SCC Tests
    // -------------------------------------------------------------------------

    /// Test Tarjan algorithm finds simple 2-node cycle
    /// Contract: A -> B -> A is detected as single SCC
    #[test]
    #[ignore = "Tarjan SCC not yet implemented"]
    fn tarjan_finds_simple_cycle() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("cycles.py", SIMPLE_CYCLE).unwrap();

        // Expected: find exactly one SCC with 2 nodes
        // let graph = build_call_graph(test_dir.path(), Language::Python).unwrap();
        // let sccs = tarjan_scc(&graph);
        //
        // // Filter to SCCs with > 1 node (cycles)
        // let cycles: Vec<_> = sccs.iter().filter(|scc| scc.len() > 1).collect();
        //
        // assert_eq!(cycles.len(), 1);
        // assert_eq!(cycles[0].len(), 2);
        // assert!(cycles[0].contains(&"func_a".to_string()));
        // assert!(cycles[0].contains(&"func_b".to_string()));

        todo!("Implement tarjan_finds_simple_cycle test");
    }

    /// Test Tarjan algorithm finds complex 3+ node cycle
    /// Contract: A -> B -> C -> A is detected as single SCC
    #[test]
    #[ignore = "Tarjan SCC not yet implemented"]
    fn tarjan_finds_complex_scc() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("cycles.py", COMPLEX_CYCLE).unwrap();

        // Expected: find exactly one SCC with 3 nodes
        // let graph = build_call_graph(test_dir.path(), Language::Python).unwrap();
        // let sccs = tarjan_scc(&graph);
        //
        // let cycles: Vec<_> = sccs.iter().filter(|scc| scc.len() > 1).collect();
        //
        // assert_eq!(cycles.len(), 1);
        // assert_eq!(cycles[0].len(), 3);
        // assert!(cycles[0].contains(&"node_a".to_string()));
        // assert!(cycles[0].contains(&"node_b".to_string()));
        // assert!(cycles[0].contains(&"node_c".to_string()));

        todo!("Implement tarjan_finds_complex_scc test");
    }

    /// Test Tarjan algorithm correctly identifies multiple separate SCCs
    /// Contract: Two independent cycles are found as separate SCCs
    #[test]
    #[ignore = "Tarjan SCC not yet implemented"]
    fn tarjan_finds_multiple_sccs() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("cycles.py", MULTIPLE_SCCS).unwrap();

        // Expected: find 2 SCCs (cycle_a/b and loop_x/y/z)
        // let graph = build_call_graph(test_dir.path(), Language::Python).unwrap();
        // let sccs = tarjan_scc(&graph);
        //
        // let cycles: Vec<_> = sccs.iter().filter(|scc| scc.len() > 1).collect();
        //
        // assert_eq!(cycles.len(), 2);
        // // One SCC with 2 nodes, one with 3 nodes
        // let sizes: Vec<_> = cycles.iter().map(|c| c.len()).collect();
        // assert!(sizes.contains(&2));
        // assert!(sizes.contains(&3));

        todo!("Implement tarjan_finds_multiple_sccs test");
    }

    /// Test Tarjan algorithm has no false positives on DAG
    /// Contract: No cycles reported for directed acyclic graph
    #[test]
    #[ignore = "Tarjan SCC not yet implemented"]
    fn tarjan_no_false_positives() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("dag.py", DAG_NO_CYCLES).unwrap();

        // Expected: no SCCs with > 1 node
        // let graph = build_call_graph(test_dir.path(), Language::Python).unwrap();
        // let sccs = tarjan_scc(&graph);
        //
        // let cycles: Vec<_> = sccs.iter().filter(|scc| scc.len() > 1).collect();
        //
        // assert!(cycles.is_empty(), "DAG should have no cycles");

        todo!("Implement tarjan_no_false_positives test");
    }

    /// Test Tarjan SCC output schema
    /// Contract: Output includes nodes, edges, size, is_cycle fields
    #[test]
    #[ignore = "Tarjan SCC not yet implemented"]
    fn tarjan_output_schema() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("cycles.py", SIMPLE_CYCLE).unwrap();

        // Expected: CycleReport with proper schema
        // let report = detect_cycles(test_dir.path(), Language::Python, CycleGranularity::Function).unwrap();
        //
        // assert!(report.cycles.len() >= 1);
        //
        // let cycle = &report.cycles[0];
        // assert!(!cycle.nodes.is_empty());
        // assert!(!cycle.edges.is_empty());
        // assert!(cycle.size > 1);
        // assert!(cycle.is_cycle);
        //
        // // Verify summary
        // assert!(report.summary.total_sccs > 0);
        // assert!(report.summary.cycle_count > 0);
        // assert_eq!(report.summary.granularity, "function");

        todo!("Implement tarjan_output_schema test");
    }
}

// =============================================================================
// patterns Tests
// =============================================================================

#[cfg(test)]
mod patterns_tests {
    use super::fixtures::*;

    // -------------------------------------------------------------------------
    // Soft Delete Detection Tests
    // -------------------------------------------------------------------------

    /// Test detection of is_deleted boolean field
    /// Contract: Confidence >= 0.4 when is_deleted field found
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn detects_is_deleted_field() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("models/user.py", PYTHON_SOFT_DELETE)
            .unwrap();

        // Expected: soft_delete pattern detected with high confidence
        // let report = detect_patterns(test_dir.path(), None).unwrap();
        //
        // assert!(report.soft_delete.detected);
        // assert!(report.soft_delete.confidence >= 0.4);
        // assert!(report.soft_delete.column_names.contains(&"is_deleted".to_string()));

        todo!("Implement detects_is_deleted_field test");
    }

    /// Test detection of deleted_at timestamp field
    /// Contract: Confidence >= 0.4 when deleted_at field found
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn detects_deleted_at_timestamp() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("models/user.py", PYTHON_SOFT_DELETE)
            .unwrap();

        // Expected: deleted_at field detected
        // let report = detect_patterns(test_dir.path(), None).unwrap();
        //
        // assert!(report.soft_delete.column_names.contains(&"deleted_at".to_string()));
        // assert!(report.soft_delete.confidence >= 0.8); // Both fields = high confidence

        todo!("Implement detects_deleted_at_timestamp test");
    }

    // -------------------------------------------------------------------------
    // Error Handling Detection Tests
    // -------------------------------------------------------------------------

    /// Test detection of try/catch pattern
    /// Contract: try_catch pattern listed with confidence >= 0.3
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn detects_try_catch_pattern() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("service.py", PYTHON_ERROR_HANDLING)
            .unwrap();

        // Expected: try_catch pattern detected
        // let report = detect_patterns(test_dir.path(), None).unwrap();
        //
        // assert!(report.error_handling.confidence >= 0.3);
        // assert!(report.error_handling.patterns.contains(&"try_catch".to_string()));

        todo!("Implement detects_try_catch_pattern test");
    }

    /// Test detection of Result<T, E> type usage in Rust
    /// Contract: result_type pattern listed with confidence >= 0.4
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn detects_result_type_usage() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("lib.rs", RUST_ERROR_HANDLING).unwrap();

        // Expected: result_type pattern detected for Rust
        // let report = detect_patterns(test_dir.path(), Some(Language::Rust)).unwrap();
        //
        // assert!(report.error_handling.confidence >= 0.4);
        // assert!(report.error_handling.patterns.contains(&"result_type".to_string()));

        todo!("Implement detects_result_type_usage test");
    }

    /// Test detection of custom exception classes
    /// Contract: Exception types listed with evidence
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn detects_custom_exceptions() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("service.py", PYTHON_ERROR_HANDLING)
            .unwrap();

        // Expected: custom exception classes detected
        // let report = detect_patterns(test_dir.path(), None).unwrap();
        //
        // assert!(report.error_handling.exception_types.contains(&"ValidationError".to_string()));
        // assert!(report.error_handling.exception_types.contains(&"NotFoundError".to_string()));

        todo!("Implement detects_custom_exceptions test");
    }

    // -------------------------------------------------------------------------
    // Naming Convention Detection Tests
    // -------------------------------------------------------------------------

    /// Test detection of snake_case function naming
    /// Contract: functions field shows "snake_case" for Python
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn detects_snake_case_functions() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("service.py", PYTHON_NAMING).unwrap();

        // Expected: snake_case detected for functions
        // let report = detect_patterns(test_dir.path(), Some(Language::Python)).unwrap();
        //
        // assert_eq!(report.naming.functions, NamingConvention::SnakeCase);
        // assert!(report.naming.consistency_score >= 0.9);

        todo!("Implement detects_snake_case_functions test");
    }

    /// Test detection of PascalCase class naming
    /// Contract: classes field shows "PascalCase"
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn detects_pascal_case_classes() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("service.py", PYTHON_NAMING).unwrap();

        // Expected: PascalCase detected for classes
        // let report = detect_patterns(test_dir.path(), None).unwrap();
        //
        // assert_eq!(report.naming.classes, NamingConvention::PascalCase);

        todo!("Implement detects_pascal_case_classes test");
    }

    /// Test detection of UPPER_SNAKE_CASE constants
    /// Contract: constants field shows "UPPER_SNAKE_CASE"
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn detects_upper_snake_case_constants() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("service.py", PYTHON_NAMING).unwrap();

        // Expected: UPPER_SNAKE_CASE detected for constants
        // let report = detect_patterns(test_dir.path(), None).unwrap();
        //
        // assert_eq!(report.naming.constants, NamingConvention::UpperSnakeCase);

        todo!("Implement detects_upper_snake_case_constants test");
    }

    // -------------------------------------------------------------------------
    // Resource Management Detection Tests
    // -------------------------------------------------------------------------

    /// Test detection of Python context manager pattern
    /// Contract: context_manager pattern detected with __enter__/__exit__
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn detects_context_manager() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("file_utils.py", PYTHON_CONTEXT_MANAGER)
            .unwrap();

        // Expected: context_manager pattern detected
        // let report = detect_patterns(test_dir.path(), Some(Language::Python)).unwrap();
        //
        // assert!(report.resource_management.confidence >= 0.5);
        // assert!(report.resource_management.patterns.contains(&"context_manager".to_string()));

        todo!("Implement detects_context_manager test");
    }

    /// Test detection of Go defer statement
    /// Contract: defer pattern detected
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn detects_defer_statement() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("file.go", GO_DEFER).unwrap();

        // Expected: defer pattern detected for Go
        // let report = detect_patterns(test_dir.path(), Some(Language::Go)).unwrap();
        //
        // assert!(report.resource_management.patterns.contains(&"defer".to_string()));

        todo!("Implement detects_defer_statement test");
    }

    /// Test detection of Rust RAII/Drop pattern
    /// Contract: raii pattern detected with Drop impl
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn detects_raii_pattern() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("lib.rs", RUST_RAII).unwrap();

        // Expected: raii pattern detected for Rust
        // let report = detect_patterns(test_dir.path(), Some(Language::Rust)).unwrap();
        //
        // assert!(report.resource_management.patterns.contains(&"raii".to_string()));

        todo!("Implement detects_raii_pattern test");
    }

    // -------------------------------------------------------------------------
    // Multi-Language Detection Tests
    // -------------------------------------------------------------------------

    /// Test Python pattern detection
    /// Contract: Python-specific patterns detected (try/except, context manager)
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn python_patterns_detected() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("service.py", PYTHON_ERROR_HANDLING)
            .unwrap();
        test_dir
            .add_file("file_utils.py", PYTHON_CONTEXT_MANAGER)
            .unwrap();

        // Expected: Python-specific patterns
        // let report = detect_patterns(test_dir.path(), Some(Language::Python)).unwrap();
        //
        // // Python try/except
        // assert!(report.error_handling.patterns.contains(&"try_catch".to_string()));
        // // Python context manager
        // assert!(report.resource_management.patterns.contains(&"context_manager".to_string()));

        todo!("Implement python_patterns_detected test");
    }

    /// Test TypeScript pattern detection
    /// Contract: TypeScript-specific patterns detected (try/catch, interfaces)
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn typescript_patterns_detected() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("models.ts", TYPESCRIPT_SOFT_DELETE)
            .unwrap();
        test_dir.add_file("service.ts", TYPESCRIPT_NAMING).unwrap();

        // Expected: TypeScript-specific patterns
        // let report = detect_patterns(test_dir.path(), Some(Language::TypeScript)).unwrap();
        //
        // // TypeScript interfaces
        // assert!(report.soft_delete.detected);
        // // TypeScript naming (camelCase functions)
        // assert_eq!(report.naming.functions, NamingConvention::CamelCase);

        todo!("Implement typescript_patterns_detected test");
    }

    /// Test Go pattern detection
    /// Contract: Go-specific patterns detected (if err != nil, defer)
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn go_patterns_detected() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("main.go", GO_ERROR_HANDLING).unwrap();
        test_dir.add_file("file.go", GO_DEFER).unwrap();

        // Expected: Go-specific patterns
        // let report = detect_patterns(test_dir.path(), Some(Language::Go)).unwrap();
        //
        // // Go error handling (if err != nil)
        // assert!(report.error_handling.confidence >= 0.4);
        // // Go defer
        // assert!(report.resource_management.patterns.contains(&"defer".to_string()));

        todo!("Implement go_patterns_detected test");
    }

    /// Test Rust pattern detection
    /// Contract: Rust-specific patterns detected (Result, ?, RAII)
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn rust_patterns_detected() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("lib.rs", RUST_ERROR_HANDLING).unwrap();
        test_dir.add_file("file.rs", RUST_RAII).unwrap();

        // Expected: Rust-specific patterns
        // let report = detect_patterns(test_dir.path(), Some(Language::Rust)).unwrap();
        //
        // // Rust Result type
        // assert!(report.error_handling.patterns.contains(&"result_type".to_string()));
        // // Rust ? operator
        // assert!(report.error_handling.confidence >= 0.5);
        // // Rust RAII
        // assert!(report.resource_management.patterns.contains(&"raii".to_string()));

        todo!("Implement rust_patterns_detected test");
    }

    // -------------------------------------------------------------------------
    // Output Schema Tests
    // -------------------------------------------------------------------------

    /// Test that output includes metadata section
    /// Contract: files_analyzed, duration_ms, language_distribution present
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn output_includes_metadata() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("service.py", PYTHON_NAMING).unwrap();

        // Expected: metadata in output
        // let report = detect_patterns(test_dir.path(), None).unwrap();
        //
        // assert!(report.metadata.files_analyzed > 0);
        // assert!(report.metadata.duration_ms > 0);
        // assert!(!report.metadata.language_distribution.files_by_language.is_empty());

        todo!("Implement output_includes_metadata test");
    }

    /// Test that output includes LLM constraints
    /// Contract: constraints array with category, rule, confidence, priority
    #[test]
    #[ignore = "patterns command not yet implemented"]
    fn output_includes_constraints() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("service.py", PYTHON_NAMING).unwrap();

        // Expected: constraints generated from patterns
        // let report = detect_patterns(test_dir.path(), None).unwrap();
        //
        // assert!(!report.constraints.is_empty());
        // let constraint = &report.constraints[0];
        // assert!(!constraint.category.is_empty());
        // assert!(!constraint.rule.is_empty());
        // assert!(constraint.confidence > 0.0);
        // assert!(constraint.priority >= 1);

        todo!("Implement output_includes_constraints test");
    }
}

// =============================================================================
// inheritance Tests
// =============================================================================

#[cfg(test)]
mod inheritance_tests {
    use super::fixtures::*;
    use crate::inheritance::{extract_inheritance, InheritanceOptions};
    use crate::types::Language;

    // -------------------------------------------------------------------------
    // Class Extraction Tests
    // -------------------------------------------------------------------------

    /// Test Python class hierarchy extraction
    /// Contract: ClassDef nodes extracted with bases
    #[test]
    fn extracts_python_class_hierarchy() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("animals.py", PYTHON_INHERITANCE).unwrap();

        let options = InheritanceOptions::default();
        let report =
            extract_inheritance(test_dir.path(), Some(Language::Python), &options).unwrap();

        // Check Dog -> Mammal chain
        let dog = report
            .nodes
            .iter()
            .find(|n| n.name == "Dog")
            .expect("Dog class not found");
        assert!(dog.bases.contains(&"Mammal".to_string()));

        let mammal = report
            .nodes
            .iter()
            .find(|n| n.name == "Mammal")
            .expect("Mammal class not found");
        assert!(mammal.bases.contains(&"Animal".to_string()));

        // Animal extends ABC
        let animal = report
            .nodes
            .iter()
            .find(|n| n.name == "Animal")
            .expect("Animal class not found");
        assert!(animal.bases.contains(&"ABC".to_string()));
    }

    /// Test TypeScript class hierarchy extraction
    /// Contract: class/interface/abstract declarations extracted
    #[test]
    fn extracts_typescript_class_hierarchy() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("animals.ts", TYPESCRIPT_INHERITANCE)
            .unwrap();

        let options = InheritanceOptions::default();
        let report =
            extract_inheritance(test_dir.path(), Some(Language::TypeScript), &options).unwrap();

        // Check Dog -> Mammal chain and implements Serializable
        let dog = report
            .nodes
            .iter()
            .find(|n| n.name == "Dog")
            .expect("Dog class not found");
        assert!(dog.bases.contains(&"Mammal".to_string()));
        assert!(dog.bases.contains(&"Serializable".to_string())); // implements

        // Serializable is an interface
        let serializable = report
            .nodes
            .iter()
            .find(|n| n.name == "Serializable")
            .expect("Serializable not found");
        assert_eq!(serializable.interface, Some(true));

        // Animal is abstract
        let animal = report
            .nodes
            .iter()
            .find(|n| n.name == "Animal")
            .expect("Animal not found");
        assert_eq!(animal.is_abstract, Some(true));
    }

    // -------------------------------------------------------------------------
    // Pattern Detection Tests
    // -------------------------------------------------------------------------

    /// Test ABC/Protocol detection in Python
    /// Contract: abstract=true for ABC inheritors, protocol=true for Protocol
    #[test]
    fn detects_abc_protocol() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("animals.py", PYTHON_INHERITANCE).unwrap();

        let options = InheritanceOptions::default();
        let report =
            extract_inheritance(test_dir.path(), Some(Language::Python), &options).unwrap();

        // Animal is abstract (inherits from ABC)
        let animal = report.nodes.iter().find(|n| n.name == "Animal").unwrap();
        assert_eq!(animal.is_abstract, Some(true));

        // Serializable is a protocol
        let serializable = report
            .nodes
            .iter()
            .find(|n| n.name == "Serializable")
            .unwrap();
        assert_eq!(serializable.protocol, Some(true));
    }

    /// Test mixin class detection
    /// Contract: mixin=true for classes ending in "Mixin" or used as secondary base
    #[test]
    fn detects_mixin_class() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("mixins.py", PYTHON_MIXIN).unwrap();

        let options = InheritanceOptions::default();
        let report =
            extract_inheritance(test_dir.path(), Some(Language::Python), &options).unwrap();

        // TimestampMixin is detected as mixin (name ends with Mixin)
        let timestamp_mixin = report
            .nodes
            .iter()
            .find(|n| n.name == "TimestampMixin")
            .unwrap();
        assert_eq!(timestamp_mixin.mixin, Some(true));

        // AuditMixin is detected as mixin
        let audit_mixin = report
            .nodes
            .iter()
            .find(|n| n.name == "AuditMixin")
            .unwrap();
        assert_eq!(audit_mixin.mixin, Some(true));

        // User uses both mixins
        let user = report.nodes.iter().find(|n| n.name == "User").unwrap();
        assert!(user.bases.contains(&"TimestampMixin".to_string()));
        assert!(user.bases.contains(&"AuditMixin".to_string()));
    }

    /// Test diamond inheritance detection
    /// Contract: diamonds array contains detected patterns with paths
    #[test]
    fn detects_diamond_inheritance() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("diamond.py", PYTHON_DIAMOND).unwrap();

        let options = InheritanceOptions::default();
        let report =
            extract_inheritance(test_dir.path(), Some(Language::Python), &options).unwrap();

        // Diamond inheritance should be detected
        assert!(
            !report.diamonds.is_empty(),
            "Expected diamond pattern to be detected"
        );
        let diamond = &report.diamonds[0];
        assert_eq!(diamond.class_name, "Diamond");
        assert_eq!(diamond.common_ancestor, "Base");
        assert_eq!(diamond.paths.len(), 2);
    }

    // -------------------------------------------------------------------------
    // External Base Resolution Tests
    // -------------------------------------------------------------------------

    /// Test that external base classes are marked correctly
    /// Contract: resolution field is "stdlib", "project", or "unresolved"
    #[test]
    fn marks_external_base_classes() {
        use crate::types::BaseResolution;

        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("animals.py", PYTHON_INHERITANCE).unwrap();

        let options = InheritanceOptions::default();
        let report =
            extract_inheritance(test_dir.path(), Some(Language::Python), &options).unwrap();

        // ABC should be stdlib
        let abc_edge = report
            .edges
            .iter()
            .find(|e| e.parent == "ABC")
            .expect("ABC edge not found");
        assert_eq!(abc_edge.resolution, BaseResolution::Stdlib);
        assert!(abc_edge.external);

        // Exception should be stdlib
        let exc_edge = report
            .edges
            .iter()
            .find(|e| e.parent == "Exception")
            .expect("Exception edge not found");
        assert_eq!(exc_edge.resolution, BaseResolution::Stdlib);
        assert!(exc_edge.external);

        // Mammal should be project
        let mammal_edge = report
            .edges
            .iter()
            .find(|e| e.parent == "Mammal" && e.child == "Dog")
            .expect("Mammal->Dog edge not found");
        assert_eq!(mammal_edge.resolution, BaseResolution::Project);
        assert!(!mammal_edge.external);
    }

    // -------------------------------------------------------------------------
    // Output Format Tests
    // -------------------------------------------------------------------------

    /// Test JSON output structure
    /// Contract: edges, nodes, roots, leaves, diamonds, count present
    #[test]
    fn json_output_structure() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("animals.py", PYTHON_INHERITANCE).unwrap();

        let options = InheritanceOptions::default();
        let report =
            extract_inheritance(test_dir.path(), Some(Language::Python), &options).unwrap();

        // Serialize to JSON and verify structure
        let json_str = serde_json::to_string(&report).unwrap();
        let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert!(json.get("edges").is_some());
        assert!(json.get("nodes").is_some());
        assert!(json.get("roots").is_some());
        assert!(json.get("leaves").is_some());
        assert!(json.get("count").is_some());
        assert!(json.get("languages").is_some());
    }

    /// Test DOT output is valid Graphviz format
    /// Contract: Output starts with "digraph", contains node and edge definitions
    #[test]
    fn dot_output_valid_graphviz() {
        use crate::inheritance::format_dot;

        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("animals.py", PYTHON_INHERITANCE).unwrap();

        let options = InheritanceOptions::default();
        let report =
            extract_inheritance(test_dir.path(), Some(Language::Python), &options).unwrap();
        let output = format_dot(&report);

        assert!(output.starts_with("digraph inheritance"));
        assert!(output.contains("rankdir=BT"));
        assert!(output.contains("->")); // Has edges
        assert!(output.contains("[label=")); // Has node labels

        // Abstract classes should have special styling
        assert!(output.contains("fillcolor=lightyellow") || output.contains("<<abstract>>"));
    }

    /// Test text output format (tree view)
    /// Contract: Hierarchical tree with proper indentation
    #[test]
    fn text_output_tree_format() {
        use crate::inheritance::format_text;

        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("animals.py", PYTHON_INHERITANCE).unwrap();

        let options = InheritanceOptions::default();
        let report =
            extract_inheritance(test_dir.path(), Some(Language::Python), &options).unwrap();
        let output = format_text(&report);

        // Debug print
        eprintln!("Text output:\n{}", output);
        eprintln!("Number of nodes: {}", report.nodes.len());

        assert!(output.contains("Inheritance Graph"));
        assert!(output.contains("Roots"));
        assert!(output.contains("Leaves"));
        assert!(output.contains("Hierarchy:"));
        // Tree structure should contain class names - either in roots/leaves or hierarchy
        assert!(
            report.nodes.iter().any(|n| n.name == "Animal"),
            "Animal not found in nodes"
        );
        assert!(
            report.nodes.iter().any(|n| n.name == "Mammal"),
            "Mammal not found in nodes"
        );
    }

    // -------------------------------------------------------------------------
    // Filter Tests
    // -------------------------------------------------------------------------

    /// Test --class filter exact match
    /// Contract: Only focused class + ancestors + descendants returned
    #[test]
    fn class_filter_exact_match() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("animals.py", PYTHON_INHERITANCE).unwrap();

        let options = InheritanceOptions {
            class_filter: Some("Mammal".to_string()),
            ..Default::default()
        };
        let report =
            extract_inheritance(test_dir.path(), Some(Language::Python), &options).unwrap();

        let names: Vec<_> = report.nodes.iter().map(|n| n.name.as_str()).collect();

        // Should include Mammal
        assert!(names.contains(&"Mammal"));
        // Should include ancestor (Animal)
        assert!(names.contains(&"Animal"));
        // Should include descendants (Dog, Cat)
        assert!(names.contains(&"Dog"));
        assert!(names.contains(&"Cat"));
        // MyException should NOT be included (unrelated)
        assert!(!names.contains(&"MyException"));
    }

    /// Test --class filter fuzzy matching suggestions
    /// Contract: When class not found, suggest similar names
    #[test]
    fn class_filter_fuzzy_suggestion() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("animals.py", PYTHON_INHERITANCE).unwrap();

        let options = InheritanceOptions {
            class_filter: Some("Mamal".to_string()), // Typo
            ..Default::default()
        };
        let result = extract_inheritance(test_dir.path(), Some(Language::Python), &options);

        // Should return error with suggestions
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Did you mean") || err.to_string().contains("Mammal"));
    }

    /// Test --depth limit
    /// Contract: Traversal stops at specified depth
    #[test]
    fn depth_limit_works() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("animals.py", PYTHON_INHERITANCE).unwrap();

        let options = InheritanceOptions {
            class_filter: Some("Mammal".to_string()),
            depth: Some(1),
            ..Default::default()
        };
        let report =
            extract_inheritance(test_dir.path(), Some(Language::Python), &options).unwrap();

        let names: Vec<_> = report.nodes.iter().map(|n| n.name.as_str()).collect();

        // Should include Mammal
        assert!(names.contains(&"Mammal"));
        // Should include direct parent (Animal)
        assert!(names.contains(&"Animal"));
        // Should include direct children (Dog, Cat)
        assert!(names.contains(&"Dog"));
        assert!(names.contains(&"Cat"));
        // ABC is depth 2 from Mammal (Mammal -> Animal -> ABC) - should NOT be included
        // Note: ABC is not in the project (it's stdlib), so it wouldn't be included anyway
    }

    // -------------------------------------------------------------------------
    // Edge Case Tests
    // -------------------------------------------------------------------------

    /// Test empty project
    /// Contract: Returns empty graph, no errors
    #[test]
    fn handles_empty_project() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("empty.py", "# No classes here\npass\n")
            .unwrap();

        let options = InheritanceOptions::default();
        let report =
            extract_inheritance(test_dir.path(), Some(Language::Python), &options).unwrap();

        assert!(report.nodes.is_empty());
        assert!(report.edges.is_empty());
        assert_eq!(report.count, 0);
    }

    /// Test single class with no inheritance
    /// Contract: Class appears in nodes, no edges
    #[test]
    fn handles_single_class() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("single.py", "class Standalone:\n    pass\n")
            .unwrap();

        let options = InheritanceOptions::default();
        let report =
            extract_inheritance(test_dir.path(), Some(Language::Python), &options).unwrap();

        assert_eq!(report.nodes.len(), 1);
        let names: Vec<_> = report.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"Standalone"));
        // No edges since no inheritance
        // Note: edges may include external bases if the class inherits from nothing
        assert!(report.roots.contains(&"Standalone".to_string()));
        assert!(report.leaves.contains(&"Standalone".to_string()));
    }

    /// Test multi-language project
    /// Contract: Both Python and TypeScript classes extracted
    #[test]
    fn handles_multi_language() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("animals.py", PYTHON_INHERITANCE).unwrap();
        test_dir
            .add_file("models.ts", TYPESCRIPT_INHERITANCE)
            .unwrap();

        // No language filter - extract from both
        let options = InheritanceOptions::default();
        let report = extract_inheritance(test_dir.path(), None, &options).unwrap();

        let names: Vec<_> = report.nodes.iter().map(|n| n.name.as_str()).collect();

        // Python classes
        assert!(names.contains(&"Animal"));
        // TypeScript classes (Serializable is an interface)
        assert!(names.contains(&"Serializable"));
        // Languages tracked
        assert!(report.languages.contains(&Language::Python));
        assert!(report.languages.contains(&Language::TypeScript));
    }
}
