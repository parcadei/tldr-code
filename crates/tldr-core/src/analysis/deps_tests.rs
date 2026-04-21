//! Test module for deps CLI command (Session 7 spec)
//!
//! These tests define expected behavior BEFORE implementation.
//! Tests are designed to FAIL until the modules are implemented.
//!
//! # Test Categories
//!
//! ## 1. Import Graph Internal tests
//! - Build internal dependency graph for Python, TypeScript, Go, Rust
//! - Empty directory and single file cases
//!
//! ## 2. Circular Dependency Detection tests
//! - Simple cycles, triangle cycles, multiple cycles
//! - Self-import handling
//!
//! ## 3. External vs Internal Dependency Classification tests
//! - Python stdlib vs project modules
//! - TypeScript node_modules vs relative imports
//! - Go module prefix matching
//! - Rust crate name matching
//!
//! ## 4. File Level vs Module Level tests
//! - Default file-level granularity
//! - Package-level with --collapse-packages
//!
//! ## 5. Transitive Dependencies tests
//! - Direct deps (depth=1)
//! - Transitive deps (depth=2,3,...)
//! - Depth limit option
//!
//! ## 6. Unused Import Detection tests
//! - Unused imports, re-exports, type-only imports
//!
//! ## 7. Dependency Visualization tests
//! - DOT format output validation
//! - Cycle highlighting
//!
//! Reference: session7-spec.md


// =============================================================================
// Test Fixture Setup Module
// =============================================================================

/// Test fixture utilities for deps analysis tests
pub mod fixtures {
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// A temporary directory for testing deps analysis
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
    // Python Fixtures for Dependency Testing
    // -------------------------------------------------------------------------

    pub const PYTHON_AUTH_IMPORTS_UTILS: &str = r#"
from utils import format_date
from db import query

def login(username: str, password: str) -> bool:
    """Authenticate user credentials."""
    user = query("SELECT * FROM users WHERE username = ?", username)
    if user:
        return True
    return False
"#;

    pub const PYTHON_UTILS_NO_DEPS: &str = r#"
def format_date(timestamp: int) -> str:
    """Format timestamp as human-readable date."""
    from datetime import datetime
    return datetime.fromtimestamp(timestamp).isoformat()
"#;

    pub const PYTHON_DB_IMPORTS_UTILS: &str = r#"
from utils import format_date

def query(sql: str, *args):
    """Execute a database query."""
    log_query(sql, format_date(0))
    return None

def log_query(sql: str, timestamp: str):
    """Log query with timestamp."""
    print(f"[{timestamp}] {sql}")
"#;

    pub const PYTHON_EXTERNAL_IMPORTS: &str = r#"
import json
import os
from typing import Dict, List
import requests
import sqlalchemy

def process():
    pass
"#;

    // Circular dependency fixtures
    pub const PYTHON_CYCLE_A: &str = r#"
from cycle_b import func_b

def func_a():
    return func_b()
"#;

    pub const PYTHON_CYCLE_B: &str = r#"
from cycle_a import func_a

def func_b():
    return func_a()
"#;

    pub const PYTHON_CYCLE_TRIANGLE_A: &str = r#"
from cycle_b import func_b

def func_a():
    return func_b()
"#;

    pub const PYTHON_CYCLE_TRIANGLE_B: &str = r#"
from cycle_c import func_c

def func_b():
    return func_c()
"#;

    pub const PYTHON_CYCLE_TRIANGLE_C: &str = r#"
from cycle_a import func_a

def func_c():
    return func_a()
"#;

    pub const PYTHON_SELF_IMPORT: &str = r#"
from self_import import helper

def main():
    return helper()

def helper():
    return 42
"#;

    // Package structure fixtures
    pub const PYTHON_PKG_INIT: &str = r#"
from .module_a import func_a
from .module_b import func_b
"#;

    pub const PYTHON_PKG_MODULE_A: &str = r#"
def func_a():
    return "a"
"#;

    pub const PYTHON_PKG_MODULE_B: &str = r#"
from .module_a import func_a

def func_b():
    return func_a() + "b"
"#;

    // -------------------------------------------------------------------------
    // TypeScript Fixtures
    // -------------------------------------------------------------------------

    pub const TS_AUTH_IMPORTS: &str = r#"
import { formatDate } from './utils';
import { query } from './db';

export function login(username: string, password: string): boolean {
    const user = query('SELECT * FROM users WHERE username = ?', username);
    return !!user;
}
"#;

    pub const TS_UTILS_NO_DEPS: &str = r#"
export function formatDate(timestamp: number): string {
    return new Date(timestamp).toISOString();
}
"#;

    pub const TS_EXTERNAL_IMPORTS: &str = r#"
import express from 'express';
import { Request, Response } from 'express';
import lodash from 'lodash';
import type { Config } from './config';

export function handler(req: Request, res: Response) {}
"#;

    pub const TS_CYCLE_A: &str = r#"
import { funcB } from './cycle_b';

export function funcA(): number {
    return funcB();
}
"#;

    pub const TS_CYCLE_B: &str = r#"
import { funcA } from './cycle_a';

export function funcB(): number {
    return funcA();
}
"#;

    // -------------------------------------------------------------------------
    // Go Fixtures
    // -------------------------------------------------------------------------

    pub const GO_AUTH_IMPORTS: &str = r#"
package auth

import (
    "myproject/utils"
    "myproject/db"
)

func Login(username, password string) bool {
    user := db.Query("SELECT * FROM users")
    utils.FormatDate(0)
    return user != nil
}
"#;

    pub const GO_UTILS_NO_DEPS: &str = r#"
package utils

import "time"

func FormatDate(timestamp int64) string {
    return time.Unix(timestamp, 0).Format(time.RFC3339)
}
"#;

    pub const GO_EXTERNAL_IMPORTS: &str = r#"
package main

import (
    "fmt"
    "os"
    "github.com/gin-gonic/gin"
    "github.com/jmoiron/sqlx"
)

func main() {
    fmt.Println("hello")
}
"#;

    // -------------------------------------------------------------------------
    // Rust Fixtures
    // -------------------------------------------------------------------------

    pub const RUST_AUTH_IMPORTS: &str = r#"
use crate::utils::format_date;
use crate::db::query;

pub fn login(username: &str, password: &str) -> bool {
    let user = query("SELECT * FROM users");
    user.is_some()
}
"#;

    pub const RUST_UTILS_NO_DEPS: &str = r#"
pub fn format_date(timestamp: i64) -> String {
    // Simple date formatting
    format!("{}", timestamp)
}
"#;

    pub const RUST_EXTERNAL_IMPORTS: &str = r#"
use std::collections::HashMap;
use std::io::Read;
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

pub fn process() {}
"#;

    pub const RUST_CYCLE_A: &str = r#"
use crate::cycle_b::func_b;

pub fn func_a() -> i32 {
    func_b()
}
"#;

    pub const RUST_CYCLE_B: &str = r#"
use crate::cycle_a::func_a;

pub fn func_b() -> i32 {
    func_a()
}
"#;

    // -------------------------------------------------------------------------
    // Unused Import Fixtures
    // -------------------------------------------------------------------------

    pub const PYTHON_UNUSED_IMPORT: &str = r#"
import os
import sys  # unused
from typing import List, Dict  # Dict is unused

def get_path() -> List[str]:
    return os.environ.get("PATH", "").split(":")
"#;

    pub const TS_UNUSED_IMPORT: &str = r#"
import { used, unused } from './utils';
import type { Config } from './config';  // type-only

export function process(): void {
    used();
}
"#;

    // -------------------------------------------------------------------------
    // Deep Chain for Transitive Testing
    // -------------------------------------------------------------------------

    pub const PYTHON_CHAIN_E: &str = r#"
def func_e():
    return 42
"#;

    pub const PYTHON_CHAIN_D: &str = r#"
from chain_e import func_e

def func_d():
    return func_e()
"#;

    pub const PYTHON_CHAIN_C: &str = r#"
from chain_d import func_d

def func_c():
    return func_d()
"#;

    pub const PYTHON_CHAIN_B: &str = r#"
from chain_c import func_c

def func_b():
    return func_c()
"#;

    pub const PYTHON_CHAIN_A: &str = r#"
from chain_b import func_b

def func_a():
    return func_b()
"#;

    // -------------------------------------------------------------------------
    // Java Fixtures for Dependency Testing
    // -------------------------------------------------------------------------

    pub const JAVA_PRECONDITIONS: &str = r#"
package com.google.common.base;

public class Preconditions {
    public static void checkNotNull(Object obj) {
        if (obj == null) {
            throw new NullPointerException();
        }
    }
}
"#;

    pub const JAVA_STRINGS: &str = r#"
package com.google.common.base;

public final class Strings {
    public static boolean isNullOrEmpty(String string) {
        return string == null || string.isEmpty();
    }
}
"#;

    pub const JAVA_JOINER: &str = r#"
package com.google.common.base;

import com.google.common.base.Preconditions;

public class Joiner {
    public static Joiner on(String separator) {
        Preconditions.checkNotNull(separator);
        return new Joiner(separator);
    }

    private final String separator;
    private Joiner(String separator) {
        this.separator = separator;
    }
}
"#;

    pub const JAVA_SPLITTER: &str = r#"
package com.google.common.base;

import com.google.common.base.Preconditions;
import com.google.common.base.Strings;

public class Splitter {
    public static Splitter on(String separator) {
        Preconditions.checkNotNull(separator);
        return new Splitter(separator);
    }

    private final String separator;
    private Splitter(String separator) {
        this.separator = separator;
    }
}
"#;

    pub const JAVA_WILDCARD_IMPORT: &str = r#"
package com.google.common.collect;

import com.google.common.base.*;

public class Lists {
    public static void checkArgs(Object arg) {
        // Uses wildcard import from base package
    }
}
"#;

    pub const JAVA_STATIC_IMPORT: &str = r#"
package com.google.common.collect;

import static com.google.common.base.Preconditions.checkNotNull;

public class Sets {
    public static void validate(Object arg) {
        checkNotNull(arg);
    }
}
"#;

    pub const JAVA_EXTERNAL_IMPORTS: &str = r#"
package com.example.app;

import java.util.List;
import java.util.Map;
import javax.annotation.Nullable;
import com.google.common.base.Preconditions;

public class App {
    public void run() {
        Preconditions.checkNotNull(null);
    }
}
"#;

    pub const JAVA_NO_DEPS: &str = r#"
package com.example.util;

public class Constants {
    public static final String VERSION = "1.0.0";
}
"#;

    pub const JAVA_CYCLE_A: &str = r#"
package com.example.cycle;

import com.example.cycle.CycleB;

public class CycleA {
    public void call() {
        new CycleB().call();
    }
}
"#;

    pub const JAVA_CYCLE_B: &str = r#"
package com.example.cycle;

import com.example.cycle.CycleA;

public class CycleB {
    public void call() {
        new CycleA().call();
    }
}
"#;
}

// =============================================================================
// Import Graph Internal Tests
// =============================================================================

#[cfg(test)]
mod import_graph_internal_tests {
    use super::fixtures::*;
    use crate::analysis::deps::analyze_dependencies;
    use std::path::PathBuf;

    /// Test building internal dependency graph for Python
    /// Contract: Nodes are files, edges are import relationships
    #[test]
    fn test_python_imports() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_IMPORTS_UTILS)
            .unwrap();
        test_dir
            .add_file("src/utils.py", PYTHON_UTILS_NO_DEPS)
            .unwrap();
        test_dir
            .add_file("src/db.py", PYTHON_DB_IMPORTS_UTILS)
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Should find all 3 files
        assert_eq!(report.stats.total_files, 3, "Expected 3 files");

        // auth.py imports utils and db
        let auth_deps = report
            .internal_dependencies
            .get(&PathBuf::from("src/auth.py"));
        assert!(auth_deps.is_some(), "auth.py should have dependencies");
        let auth_deps = auth_deps.unwrap();
        assert!(
            auth_deps.contains(&PathBuf::from("src/utils.py")),
            "auth.py should import utils.py, got {:?}",
            auth_deps
        );
        assert!(
            auth_deps.contains(&PathBuf::from("src/db.py")),
            "auth.py should import db.py, got {:?}",
            auth_deps
        );

        // db.py imports utils
        let db_deps = report
            .internal_dependencies
            .get(&PathBuf::from("src/db.py"));
        assert!(db_deps.is_some(), "db.py should have dependencies");
        let db_deps = db_deps.unwrap();
        assert!(
            db_deps.contains(&PathBuf::from("src/utils.py")),
            "db.py should import utils.py, got {:?}",
            db_deps
        );

        // utils.py has no internal deps (datetime is stdlib)
        let utils_deps = report
            .internal_dependencies
            .get(&PathBuf::from("src/utils.py"));
        assert!(utils_deps.is_some(), "utils.py should be in report");
        let utils_deps = utils_deps.unwrap();
        assert!(
            utils_deps.is_empty(),
            "utils.py should have no internal deps, got {:?}",
            utils_deps
        );
    }

    /// Test building internal dependency graph for Python (legacy test name kept for compatibility)
    #[test]
    #[ignore = "covered by test_python_imports"]
    fn python_import_graph() {
        // This test is superseded by test_python_imports above
        todo!("Use test_python_imports instead")
    }

    /// Test building internal dependency graph for TypeScript
    /// Contract: Handles relative imports (./), path aliases
    #[test]
    #[ignore = "deps module not yet implemented"]
    fn typescript_import_graph() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/auth.ts", TS_AUTH_IMPORTS).unwrap();
        test_dir.add_file("src/utils.ts", TS_UTILS_NO_DEPS).unwrap();
        test_dir
            .add_file("src/db.ts", "export function query(sql: string) {}")
            .unwrap();

        // Expected: auth -> utils, auth -> db
        // let report = analyze_dependencies(
        //     test_dir.path(),
        //     Language::TypeScript,
        //     &DepsConfig::default(),
        // ).unwrap();
        //
        // assert_eq!(report.stats.total_files, 3);
        // let auth_deps = report.internal_dependencies.get(&PathBuf::from("src/auth.ts"));
        // assert!(auth_deps.is_some());

        todo!("Implement typescript_import_graph test")
    }

    /// Test building internal dependency graph for Go
    /// Contract: Uses module path prefix matching
    #[test]
    #[ignore = "deps module not yet implemented"]
    fn go_import_graph() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("go.mod", "module myproject\n\ngo 1.21")
            .unwrap();
        test_dir.add_file("auth/auth.go", GO_AUTH_IMPORTS).unwrap();
        test_dir
            .add_file("utils/utils.go", GO_UTILS_NO_DEPS)
            .unwrap();
        test_dir
            .add_file(
                "db/db.go",
                "package db\n\nfunc Query(sql string) interface{} { return nil }",
            )
            .unwrap();

        // Expected: auth -> utils, auth -> db
        // let report = analyze_dependencies(
        //     test_dir.path(),
        //     Language::Go,
        //     &DepsConfig::default(),
        // ).unwrap();
        //
        // assert!(report.stats.total_files >= 3);

        todo!("Implement go_import_graph test")
    }

    /// Test building internal dependency graph for Rust
    /// Contract: Handles crate::, super::, self:: imports
    #[test]
    #[ignore = "deps module not yet implemented"]
    fn rust_import_graph() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/lib.rs", "pub mod auth;\npub mod utils;\npub mod db;")
            .unwrap();
        test_dir.add_file("src/auth.rs", RUST_AUTH_IMPORTS).unwrap();
        test_dir
            .add_file("src/utils.rs", RUST_UTILS_NO_DEPS)
            .unwrap();
        test_dir
            .add_file(
                "src/db.rs",
                "pub fn query(_sql: &str) -> Option<()> { None }",
            )
            .unwrap();

        // Expected: auth -> utils, auth -> db
        // let report = analyze_dependencies(
        //     test_dir.path(),
        //     Language::Rust,
        //     &DepsConfig::default(),
        // ).unwrap();
        //
        // assert!(report.stats.total_files >= 3);

        todo!("Implement rust_import_graph test")
    }

    /// Test empty directory returns empty report
    /// Contract: Returns report with zero stats, no errors
    #[test]
    fn test_empty_directory() {
        let test_dir = TestDir::new().unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        assert_eq!(report.stats.total_files, 0, "Empty dir should have 0 files");
        assert_eq!(
            report.stats.total_internal_deps, 0,
            "Empty dir should have 0 deps"
        );
        assert!(
            report.internal_dependencies.is_empty(),
            "Empty dir should have no dependencies"
        );
    }

    /// Test single file with no imports
    /// Contract: Returns report with one file, zero edges
    #[test]
    fn test_single_file_no_imports() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.py", "def main():\n    print('hello')")
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        assert_eq!(report.stats.total_files, 1, "Should find 1 file");
        assert_eq!(
            report.stats.total_internal_deps, 0,
            "File with no imports should have 0 internal deps"
        );
    }

    /// Test files with parse errors are skipped
    /// Contract: Recoverable parse errors don't fail the entire analysis
    #[test]
    #[ignore = "deps module not yet implemented"]
    fn skips_parse_errors() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/valid.py", PYTHON_UTILS_NO_DEPS)
            .unwrap();
        test_dir
            .add_file("src/invalid.py", "def broken(\n    # syntax error")
            .unwrap();

        // let report = analyze_dependencies(
        //     test_dir.path(),
        //     Language::Python,
        //     &DepsConfig::default(),
        // ).unwrap();
        //
        // // Should process valid file despite invalid file existing
        // assert!(report.stats.total_files >= 1);

        todo!("Implement skips_parse_errors test")
    }
}

// =============================================================================
// Circular Dependency Detection Tests
// =============================================================================

#[cfg(test)]
mod circular_dependency_tests {
    use super::fixtures::*;
    use crate::analysis::deps::analyze_dependencies;
    

    /// Test simple cycle detection: A -> B -> A
    /// Contract: Detects 2-node cycle with correct nodes
    #[test]
    fn test_simple_cycle() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/cycle_a.py", PYTHON_CYCLE_A).unwrap();
        test_dir.add_file("src/cycle_b.py", PYTHON_CYCLE_B).unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        assert_eq!(
            report.stats.cycles_found, 1,
            "Expected 1 cycle, found {}. Cycles: {:?}",
            report.stats.cycles_found, report.circular_dependencies
        );
        let cycle = &report.circular_dependencies[0];
        assert_eq!(
            cycle.length, 2,
            "Expected cycle of length 2, got {}",
            cycle.length
        );
    }

    /// Test triangle cycle detection: A -> B -> C -> A
    /// Contract: Detects 3-node cycle
    #[test]
    fn test_triangle_cycle() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/cycle_a.py", PYTHON_CYCLE_TRIANGLE_A)
            .unwrap();
        test_dir
            .add_file("src/cycle_b.py", PYTHON_CYCLE_TRIANGLE_B)
            .unwrap();
        test_dir
            .add_file("src/cycle_c.py", PYTHON_CYCLE_TRIANGLE_C)
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        assert_eq!(
            report.stats.cycles_found, 1,
            "Expected 1 cycle, found {}. Cycles: {:?}",
            report.stats.cycles_found, report.circular_dependencies
        );
        let cycle = &report.circular_dependencies[0];
        assert_eq!(
            cycle.length, 3,
            "Expected cycle of length 3, got {}",
            cycle.length
        );
    }

    /// Test multiple cycles in same codebase
    /// Contract: All cycles are reported
    #[test]
    fn test_multiple_cycles() {
        let test_dir = TestDir::new().unwrap();
        // Cycle 1: a <-> b
        test_dir.add_file("src/cycle_a.py", PYTHON_CYCLE_A).unwrap();
        test_dir.add_file("src/cycle_b.py", PYTHON_CYCLE_B).unwrap();
        // Cycle 2: x <-> y
        test_dir
            .add_file(
                "src/cycle_x.py",
                "from cycle_y import func_y\n\ndef func_x(): return func_y()",
            )
            .unwrap();
        test_dir
            .add_file(
                "src/cycle_y.py",
                "from cycle_x import func_x\n\ndef func_y(): return func_x()",
            )
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        assert_eq!(
            report.stats.cycles_found, 2,
            "Expected 2 cycles, found {}. Cycles: {:?}",
            report.stats.cycles_found, report.circular_dependencies
        );
    }

    /// Test no cycles case
    /// Contract: cycles_found = 0, circular_dependencies is empty
    #[test]
    fn test_no_cycles() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_IMPORTS_UTILS)
            .unwrap();
        test_dir
            .add_file("src/utils.py", PYTHON_UTILS_NO_DEPS)
            .unwrap();
        test_dir
            .add_file("src/db.py", PYTHON_DB_IMPORTS_UTILS)
            .unwrap();

        // DAG structure: auth -> {utils, db}, db -> utils
        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        assert_eq!(
            report.stats.cycles_found, 0,
            "Expected 0 cycles, found {}. Cycles: {:?}",
            report.stats.cycles_found, report.circular_dependencies
        );
        assert!(
            report.circular_dependencies.is_empty(),
            "Expected no circular dependencies"
        );
    }

    /// Test self-import case
    /// Contract: Self-imports within same file are NOT counted as cycles
    #[test]
    fn test_self_import() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/self_import.py", PYTHON_SELF_IMPORT)
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Self-import should not be counted as a cycle
        // (self-imports are filtered out in analyze_dependencies)
        assert_eq!(
            report.stats.cycles_found, 0,
            "Self-import should not create a cycle. Found: {:?}",
            report.circular_dependencies
        );
    }

    /// Test max_cycle_length limits reported cycles
    /// Contract: Cycles longer than max_cycle_length are not reported
    #[test]
    fn test_max_cycle_length_limit() {
        let test_dir = TestDir::new().unwrap();
        // Create a 5-node cycle: a -> b -> c -> d -> e -> a
        test_dir
            .add_file("src/cyc_a.py", "from cyc_b import f\ndef f(): return f()")
            .unwrap();
        test_dir
            .add_file("src/cyc_b.py", "from cyc_c import f\ndef f(): return f()")
            .unwrap();
        test_dir
            .add_file("src/cyc_c.py", "from cyc_d import f\ndef f(): return f()")
            .unwrap();
        test_dir
            .add_file("src/cyc_d.py", "from cyc_e import f\ndef f(): return f()")
            .unwrap();
        test_dir
            .add_file("src/cyc_e.py", "from cyc_a import f\ndef f(): return f()")
            .unwrap();

        let mut options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };
        options.max_cycle_length = Some(3); // Only report cycles <= 3 nodes

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // 5-node cycle should be excluded with max_cycle_length=3
        assert_eq!(
            report.stats.cycles_found, 0,
            "5-node cycle should be excluded with max_cycle_length=3. Found: {:?}",
            report.circular_dependencies
        );
    }

    /// Test TypeScript cycle detection
    /// Contract: Works with relative imports in TypeScript
    #[test]
    #[ignore = "TypeScript import resolution needs more work"]
    fn test_typescript_cycle_detection() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/cycle_a.ts", TS_CYCLE_A).unwrap();
        test_dir.add_file("src/cycle_b.ts", TS_CYCLE_B).unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("typescript".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        assert_eq!(
            report.stats.cycles_found, 1,
            "Expected 1 cycle in TypeScript. Found: {:?}",
            report.circular_dependencies
        );
    }

    /// Test Rust cycle detection
    /// Contract: Works with crate:: imports in Rust
    #[test]
    #[ignore = "Rust import resolution needs more work"]
    fn test_rust_cycle_detection() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/lib.rs", "pub mod cycle_a;\npub mod cycle_b;")
            .unwrap();
        test_dir.add_file("src/cycle_a.rs", RUST_CYCLE_A).unwrap();
        test_dir.add_file("src/cycle_b.rs", RUST_CYCLE_B).unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("rust".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        assert_eq!(
            report.stats.cycles_found, 1,
            "Expected 1 cycle in Rust. Found: {:?}",
            report.circular_dependencies
        );
    }
}

// =============================================================================
// External vs Internal Dependency Tests
// =============================================================================

#[cfg(test)]
mod external_internal_tests {
    use super::fixtures::*;
    use crate::analysis::deps::{analyze_dependencies, is_python_stdlib, DepsOptions};
    use std::path::PathBuf;

    /// Test Python stdlib detection
    /// Contract: os, sys, json, etc. are classified as Stdlib
    #[test]
    fn python_stdlib_classification() {
        // Test the is_python_stdlib function directly
        assert!(is_python_stdlib("os"), "os should be stdlib");
        assert!(is_python_stdlib("sys"), "sys should be stdlib");
        assert!(is_python_stdlib("json"), "json should be stdlib");
        assert!(is_python_stdlib("typing"), "typing should be stdlib");
        assert!(
            is_python_stdlib("collections"),
            "collections should be stdlib"
        );
        assert!(is_python_stdlib("functools"), "functools should be stdlib");
        assert!(is_python_stdlib("pathlib"), "pathlib should be stdlib");
        assert!(is_python_stdlib("datetime"), "datetime should be stdlib");

        // Dotted stdlib imports
        assert!(is_python_stdlib("os.path"), "os.path should be stdlib");
        assert!(
            is_python_stdlib("collections.abc"),
            "collections.abc should be stdlib"
        );
        assert!(
            is_python_stdlib("typing.Dict"),
            "typing.Dict should be stdlib"
        );

        // External packages should NOT be classified as stdlib
        assert!(
            !is_python_stdlib("requests"),
            "requests should NOT be stdlib"
        );
        assert!(
            !is_python_stdlib("sqlalchemy"),
            "sqlalchemy should NOT be stdlib"
        );
        assert!(!is_python_stdlib("numpy"), "numpy should NOT be stdlib");
        assert!(!is_python_stdlib("pandas"), "pandas should NOT be stdlib");

        // Now test via analyze_dependencies
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.py", PYTHON_EXTERNAL_IMPORTS)
            .unwrap();

        let options = DepsOptions {
            include_external: true,
            language: Some("python".to_string()),
            ..Default::default()
        };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Collect all external dep names across all files
        let mut external_names: Vec<&str> = Vec::new();
        for deps in report.external_dependencies.values() {
            for dep in deps {
                external_names.push(dep.as_str());
            }
        }

        // requests, sqlalchemy should be in external_dependencies
        // (json, os, typing are stdlib and should also be tracked when include_external=true)
        assert!(
            external_names.contains(&"requests"),
            "requests should be in external deps. Found: {:?}",
            external_names
        );
        assert!(
            external_names.contains(&"sqlalchemy"),
            "sqlalchemy should be in external deps. Found: {:?}",
            external_names
        );
    }

    /// Test Python project module detection
    /// Contract: Relative imports and project modules are Internal
    #[test]
    fn python_internal_classification() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_IMPORTS_UTILS)
            .unwrap();
        test_dir
            .add_file("src/utils.py", PYTHON_UTILS_NO_DEPS)
            .unwrap();
        test_dir
            .add_file("src/db.py", "def query(sql): pass")
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // auth.py should be in internal_dependencies
        let auth_path = PathBuf::from("src/auth.py");
        assert!(
            report.internal_dependencies.contains_key(&auth_path),
            "src/auth.py should be in internal dependencies. Keys: {:?}",
            report.internal_dependencies.keys().collect::<Vec<_>>()
        );

        // auth.py should have utils and db as dependencies
        let auth_deps = report.internal_dependencies.get(&auth_path).unwrap();
        assert!(
            auth_deps
                .iter()
                .any(|p| p.to_string_lossy().contains("utils")),
            "auth.py should depend on utils. Found: {:?}",
            auth_deps
        );
        assert!(
            auth_deps.iter().any(|p| p.to_string_lossy().contains("db")),
            "auth.py should depend on db. Found: {:?}",
            auth_deps
        );

        // Total internal deps should be 2 (utils + db from auth.py)
        assert_eq!(
            report.stats.total_internal_deps, 2,
            "Expected 2 internal deps. Got: {}",
            report.stats.total_internal_deps
        );
    }

    /// Test TypeScript node_modules vs relative imports
    /// Contract: node_modules packages are External, relative are Internal
    #[test]
    fn typescript_external_classification() {
        use crate::analysis::deps::{is_typescript_external, is_typescript_relative};

        // Test classification functions directly
        assert!(
            is_typescript_external("express"),
            "express should be external"
        );
        assert!(
            is_typescript_external("lodash"),
            "lodash should be external"
        );
        assert!(
            is_typescript_external("@types/node"),
            "@types/node should be external"
        );

        assert!(
            is_typescript_relative("./config"),
            "./config should be relative"
        );
        assert!(
            is_typescript_relative("../utils"),
            "../utils should be relative"
        );
        assert!(
            !is_typescript_relative("express"),
            "express should NOT be relative"
        );

        // Test via analyze_dependencies
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.ts", TS_EXTERNAL_IMPORTS)
            .unwrap();
        test_dir
            .add_file("src/config.ts", "export interface Config {}")
            .unwrap();

        let options = DepsOptions {
            include_external: true,
            language: Some("typescript".to_string()),
            ..Default::default()
        };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Collect all external dep names
        let mut external_names: Vec<&str> = Vec::new();
        for deps in report.external_dependencies.values() {
            for dep in deps {
                external_names.push(dep.as_str());
            }
        }

        // express, lodash should be external
        assert!(
            external_names.contains(&"express"),
            "express should be in external deps. Found: {:?}",
            external_names
        );
        assert!(
            external_names.contains(&"lodash"),
            "lodash should be in external deps. Found: {:?}",
            external_names
        );

        // ./config should be internal (resolved)
        let main_path = PathBuf::from("src/main.ts");
        if let Some(main_deps) = report.internal_dependencies.get(&main_path) {
            assert!(
                main_deps
                    .iter()
                    .any(|p| p.to_string_lossy().contains("config")),
                "./config should be in internal deps. Found: {:?}",
                main_deps
            );
        }
    }

    /// Test Go module prefix matching
    /// Contract: Same module prefix is Internal, different is External
    #[test]
    #[ignore = "deps module not yet implemented"]
    fn go_module_prefix_classification() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("go.mod", "module myproject\n\ngo 1.21")
            .unwrap();
        test_dir.add_file("main.go", GO_EXTERNAL_IMPORTS).unwrap();

        // let config = DepsConfig {
        //     include_external: true,
        //     ..Default::default()
        // };
        // let report = analyze_dependencies(
        //     test_dir.path(),
        //     Language::Go,
        //     &config,
        // ).unwrap();
        //
        // // github.com/* packages are external
        // let external_names: Vec<_> = report.external_dependencies.keys().collect();
        // assert!(external_names.iter().any(|n| n.contains("gin-gonic")));

        todo!("Implement go_module_prefix_classification test")
    }

    /// Test Rust crate classification
    /// Contract: std is Stdlib, crate:: is Internal, others are External
    #[test]
    #[ignore = "deps module not yet implemented"]
    fn rust_crate_classification() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.rs", RUST_EXTERNAL_IMPORTS)
            .unwrap();

        // let config = DepsConfig {
        //     include_external: true,
        //     ..Default::default()
        // };
        // let report = analyze_dependencies(
        //     test_dir.path(),
        //     Language::Rust,
        //     &config,
        // ).unwrap();
        //
        // // serde, tokio are external
        // let external_names: Vec<_> = report.external_dependencies.keys().collect();
        // assert!(external_names.contains(&&"serde".to_string()));
        // assert!(external_names.contains(&&"tokio".to_string()));

        todo!("Implement rust_crate_classification test")
    }

    /// Test include_external flag disabled
    /// Contract: External dependencies not included when flag is false
    #[test]
    fn exclude_external_deps() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.py", PYTHON_EXTERNAL_IMPORTS)
            .unwrap();

        // Test with include_external = false (default)
        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };
        // include_external defaults to false

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // external_dependencies should be empty when include_external is false
        assert!(
            report.external_dependencies.is_empty(),
            "external_dependencies should be empty when include_external=false. Found: {:?}",
            report.external_dependencies
        );

        // Now test with include_external = true
        let options_with_external = DepsOptions {
            include_external: true,
            language: Some("python".to_string()),
            ..Default::default()
        };

        let report_with_external =
            analyze_dependencies(test_dir.path(), &options_with_external).unwrap();

        // external_dependencies should NOT be empty when include_external is true
        assert!(
            !report_with_external.external_dependencies.is_empty(),
            "external_dependencies should NOT be empty when include_external=true"
        );
    }

    /// Test external dependency import count
    /// Contract: import_count reflects number of import statements
    #[test]
    #[ignore = "deps module not yet implemented"]
    fn external_import_count() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/a.py", "import requests\nfrom requests import get")
            .unwrap();
        test_dir.add_file("src/b.py", "import requests").unwrap();

        // let config = DepsConfig {
        //     include_external: true,
        //     ..Default::default()
        // };
        // let report = analyze_dependencies(
        //     test_dir.path(),
        //     Language::Python,
        //     &config,
        // ).unwrap();
        //
        // let requests_dep = report.external_dependencies.get("requests").unwrap();
        // assert_eq!(requests_dep.import_count, 3);  // 2 in a.py, 1 in b.py
        // assert_eq!(requests_dep.imported_by.len(), 2);

        todo!("Implement external_import_count test")
    }
}

// =============================================================================
// File Level vs Module Level Tests
// =============================================================================

#[cfg(test)]
mod granularity_tests {
    use super::fixtures::*;
    use crate::analysis::deps::analyze_dependencies;
    

    /// Test default file-level granularity
    /// Contract: Each source file is a separate node
    #[test]
    fn file_level_default() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("pkg/__init__.py", PYTHON_PKG_INIT)
            .unwrap();
        test_dir
            .add_file("pkg/module_a.py", PYTHON_PKG_MODULE_A)
            .unwrap();
        test_dir
            .add_file("pkg/module_b.py", PYTHON_PKG_MODULE_B)
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Should have 3 nodes (one per file)
        assert_eq!(report.stats.total_files, 3, "Should find 3 files");

        // Each file should be a separate key
        let keys: Vec<_> = report.internal_dependencies.keys().collect();
        assert!(
            keys.iter()
                .any(|p| p.to_string_lossy().contains("__init__.py")),
            "__init__.py should be in keys. Got: {:?}",
            keys
        );
        assert!(
            keys.iter()
                .any(|p| p.to_string_lossy().contains("module_a.py")),
            "module_a.py should be in keys. Got: {:?}",
            keys
        );
        assert!(
            keys.iter()
                .any(|p| p.to_string_lossy().contains("module_b.py")),
            "module_b.py should be in keys. Got: {:?}",
            keys
        );
    }

    /// Test package-level with --collapse-packages
    /// Contract: Files in same package collapsed to single node
    #[test]
    fn package_level_collapsed() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("pkg/__init__.py", PYTHON_PKG_INIT)
            .unwrap();
        test_dir
            .add_file("pkg/module_a.py", PYTHON_PKG_MODULE_A)
            .unwrap();
        test_dir
            .add_file("pkg/module_b.py", PYTHON_PKG_MODULE_B)
            .unwrap();
        test_dir.add_file("other/__init__.py", "").unwrap();
        test_dir
            .add_file("other/helper.py", "from pkg import func_a")
            .unwrap();

        let mut options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };
        options.collapse_packages = true;

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Should have 2 package nodes (pkg, other)
        // internal deps within pkg are collapsed
        let node_paths: Vec<_> = report.internal_dependencies.keys().collect();
        assert!(
            node_paths
                .iter()
                .any(|p| p.to_string_lossy().ends_with("pkg")),
            "Should have 'pkg' package node. Got: {:?}",
            node_paths
        );
        assert!(
            node_paths
                .iter()
                .any(|p| p.to_string_lossy().ends_with("other")),
            "Should have 'other' package node. Got: {:?}",
            node_paths
        );

        // other -> pkg dependency should exist (cross-package)
        let other_path = node_paths
            .iter()
            .find(|p| p.to_string_lossy().ends_with("other"));
        if let Some(other_path) = other_path {
            let other_deps = report.internal_dependencies.get(*other_path);
            assert!(other_deps.is_some(), "'other' package should have deps");
            let other_deps = other_deps.unwrap();
            assert!(
                other_deps
                    .iter()
                    .any(|p| p.to_string_lossy().ends_with("pkg")),
                "'other' should depend on 'pkg'. Got: {:?}",
                other_deps
            );
        }
    }

    /// Test mixed granularity (files outside packages stay as files)
    /// Contract: Only grouped files become package nodes
    #[test]
    fn mixed_granularity() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("pkg/__init__.py", PYTHON_PKG_INIT)
            .unwrap();
        test_dir
            .add_file("pkg/module_a.py", PYTHON_PKG_MODULE_A)
            .unwrap();
        test_dir
            .add_file("main.py", "from pkg import func_a")
            .unwrap();

        let mut options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };
        options.collapse_packages = true;

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // When collapsed, files are grouped by directory
        // main.py is at root (""), pkg files are in "pkg"
        let node_paths: Vec<_> = report.internal_dependencies.keys().collect();

        // Should have entries (collapsed by directory)
        // Root-level files map to empty string or "." depending on implementation
        // pkg directory files map to "pkg"
        assert!(
            node_paths
                .iter()
                .any(|p| p.to_string_lossy().ends_with("pkg")),
            "Should have 'pkg' package node. Got: {:?}",
            node_paths
        );
    }

    /// Test that collapse_packages removes intra-package dependencies
    /// Contract: Dependencies within the same package are not included
    #[test]
    fn collapse_removes_intra_package_deps() {
        let test_dir = TestDir::new().unwrap();
        // module_b imports module_a (within same package)
        test_dir
            .add_file("pkg/__init__.py", PYTHON_PKG_INIT)
            .unwrap();
        test_dir
            .add_file("pkg/module_a.py", PYTHON_PKG_MODULE_A)
            .unwrap();
        test_dir
            .add_file("pkg/module_b.py", PYTHON_PKG_MODULE_B)
            .unwrap();

        let mut options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };
        options.collapse_packages = true;

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // When collapsed, pkg should have NO self-dependencies
        let pkg_path = report
            .internal_dependencies
            .keys()
            .find(|p| p.to_string_lossy().ends_with("pkg"));

        if let Some(pkg_path) = pkg_path {
            let pkg_deps = report.internal_dependencies.get(pkg_path).unwrap();
            assert!(
                !pkg_deps
                    .iter()
                    .any(|p| p.to_string_lossy().ends_with("pkg")),
                "Package should not depend on itself. Got: {:?}",
                pkg_deps
            );
        }
    }
}

// =============================================================================
// Transitive Dependencies Tests
// =============================================================================

#[cfg(test)]
mod transitive_deps_tests {
    use super::fixtures::*;
    use crate::analysis::deps::{analyze_dependencies, compute_transitive_deps};
    use std::path::PathBuf;

    /// Test direct dependencies (depth=1)
    /// Contract: Only immediate imports included
    #[test]
    fn direct_deps_only() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/chain_a.py", PYTHON_CHAIN_A).unwrap();
        test_dir.add_file("src/chain_b.py", PYTHON_CHAIN_B).unwrap();
        test_dir.add_file("src/chain_c.py", PYTHON_CHAIN_C).unwrap();
        test_dir.add_file("src/chain_d.py", PYTHON_CHAIN_D).unwrap();
        test_dir.add_file("src/chain_e.py", PYTHON_CHAIN_E).unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // chain_a should only show chain_b as direct dependency
        let a_deps = report
            .internal_dependencies
            .get(&PathBuf::from("src/chain_a.py"));
        assert!(a_deps.is_some(), "chain_a.py should have dependencies");
        let a_deps = a_deps.unwrap();
        assert_eq!(
            a_deps.len(),
            1,
            "chain_a should have exactly 1 direct dep, got {:?}",
            a_deps
        );
        assert!(
            a_deps
                .iter()
                .any(|p| p.to_string_lossy().contains("chain_b")),
            "chain_a should depend on chain_b, got {:?}",
            a_deps
        );
    }

    /// Test transitive dependencies at depth=2
    /// Contract: Direct + one level of transitive
    #[test]
    fn transitive_depth_2() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/chain_a.py", PYTHON_CHAIN_A).unwrap();
        test_dir.add_file("src/chain_b.py", PYTHON_CHAIN_B).unwrap();
        test_dir.add_file("src/chain_c.py", PYTHON_CHAIN_C).unwrap();
        test_dir.add_file("src/chain_d.py", PYTHON_CHAIN_D).unwrap();
        test_dir.add_file("src/chain_e.py", PYTHON_CHAIN_E).unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Use compute_transitive_deps to get transitive closure at depth 2
        let transitive = compute_transitive_deps(&report.internal_dependencies, Some(2));

        // chain_a should reach chain_b (depth 1) and chain_c (depth 2)
        let a_path = PathBuf::from("src/chain_a.py");
        let a_transitive = transitive.get(&a_path);
        assert!(
            a_transitive.is_some(),
            "chain_a should have transitive deps"
        );
        let a_transitive = a_transitive.unwrap();

        // Check that chain_b is at depth 1
        let b_path = a_transitive
            .keys()
            .find(|p| p.to_string_lossy().contains("chain_b"));
        assert!(
            b_path.is_some(),
            "chain_b should be in transitive deps of chain_a"
        );
        assert_eq!(
            a_transitive.get(b_path.unwrap()),
            Some(&1),
            "chain_b should be at depth 1"
        );

        // Check that chain_c is at depth 2
        let c_path = a_transitive
            .keys()
            .find(|p| p.to_string_lossy().contains("chain_c"));
        assert!(
            c_path.is_some(),
            "chain_c should be in transitive deps of chain_a at depth 2"
        );
        assert_eq!(
            a_transitive.get(c_path.unwrap()),
            Some(&2),
            "chain_c should be at depth 2"
        );

        // chain_d and chain_e should NOT be included at depth 2
        let d_path = a_transitive
            .keys()
            .find(|p| p.to_string_lossy().contains("chain_d"));
        assert!(
            d_path.is_none(),
            "chain_d should NOT be in transitive deps at depth 2"
        );
    }

    /// Test max_depth calculation
    /// Contract: stats.max_depth reports longest dependency chain
    #[test]
    fn max_depth_stat() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/chain_a.py", PYTHON_CHAIN_A).unwrap();
        test_dir.add_file("src/chain_b.py", PYTHON_CHAIN_B).unwrap();
        test_dir.add_file("src/chain_c.py", PYTHON_CHAIN_C).unwrap();
        test_dir.add_file("src/chain_d.py", PYTHON_CHAIN_D).unwrap();
        test_dir.add_file("src/chain_e.py", PYTHON_CHAIN_E).unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Chain: a -> b -> c -> d -> e
        // Max depth = 4 (4 edges from a to e)
        assert_eq!(
            report.stats.max_depth, 4,
            "Max depth should be 4 for chain a->b->c->d->e, got {}",
            report.stats.max_depth
        );
    }

    /// Test leaf and root file stats
    /// Contract: leaf_files have no outgoing deps, root_files have no incoming deps
    #[test]
    fn leaf_root_stats() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/chain_a.py", PYTHON_CHAIN_A).unwrap();
        test_dir.add_file("src/chain_b.py", PYTHON_CHAIN_B).unwrap();
        test_dir.add_file("src/chain_c.py", PYTHON_CHAIN_C).unwrap();
        test_dir.add_file("src/chain_d.py", PYTHON_CHAIN_D).unwrap();
        test_dir.add_file("src/chain_e.py", PYTHON_CHAIN_E).unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // chain_e is a leaf (no outgoing deps)
        // chain_a is a root (no incoming deps)
        assert_eq!(
            report.stats.leaf_files, 1,
            "Should have 1 leaf file (chain_e). Got: {}",
            report.stats.leaf_files
        );
        assert_eq!(
            report.stats.root_files, 1,
            "Should have 1 root file (chain_a). Got: {}",
            report.stats.root_files
        );
    }

    /// Test unlimited depth (default)
    /// Contract: All transitive dependencies included when max_depth is None
    #[test]
    fn unlimited_depth() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/chain_a.py", PYTHON_CHAIN_A).unwrap();
        test_dir.add_file("src/chain_b.py", PYTHON_CHAIN_B).unwrap();
        test_dir.add_file("src/chain_c.py", PYTHON_CHAIN_C).unwrap();
        test_dir.add_file("src/chain_d.py", PYTHON_CHAIN_D).unwrap();
        test_dir.add_file("src/chain_e.py", PYTHON_CHAIN_E).unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Use compute_transitive_deps with no limit
        let transitive = compute_transitive_deps(&report.internal_dependencies, None);

        // chain_a should reach all other files
        let a_path = PathBuf::from("src/chain_a.py");
        let a_transitive = transitive.get(&a_path);
        assert!(
            a_transitive.is_some(),
            "chain_a should have transitive deps"
        );
        let a_transitive = a_transitive.unwrap();

        // All files should be reachable from chain_a
        assert_eq!(
            a_transitive.len(),
            4,
            "chain_a should reach 4 other files (b, c, d, e). Got: {:?}",
            a_transitive.keys().collect::<Vec<_>>()
        );

        // Check distances
        let b_dist = a_transitive
            .iter()
            .find(|(p, _)| p.to_string_lossy().contains("chain_b"))
            .map(|(_, d)| *d);
        let c_dist = a_transitive
            .iter()
            .find(|(p, _)| p.to_string_lossy().contains("chain_c"))
            .map(|(_, d)| *d);
        let d_dist = a_transitive
            .iter()
            .find(|(p, _)| p.to_string_lossy().contains("chain_d"))
            .map(|(_, d)| *d);
        let e_dist = a_transitive
            .iter()
            .find(|(p, _)| p.to_string_lossy().contains("chain_e"))
            .map(|(_, d)| *d);

        assert_eq!(b_dist, Some(1), "chain_b should be at depth 1");
        assert_eq!(c_dist, Some(2), "chain_c should be at depth 2");
        assert_eq!(d_dist, Some(3), "chain_d should be at depth 3");
        assert_eq!(e_dist, Some(4), "chain_e should be at depth 4");
    }
}

// =============================================================================
// Unused Import Detection Tests
// =============================================================================

#[cfg(test)]
mod unused_import_tests {
    use super::fixtures::*;

    /// Test detecting unused Python imports
    /// Contract: sys and Dict are unused in the fixture
    #[test]
    #[ignore = "unused import detection not yet implemented"]
    fn python_unused_imports() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.py", PYTHON_UNUSED_IMPORT)
            .unwrap();

        // let unused = detect_unused_imports(
        //     &test_dir.path().join("src/main.py"),
        //     Language::Python,
        // ).unwrap();
        //
        // assert!(unused.iter().any(|i| i.module == "sys"));
        // assert!(unused.iter().any(|i| i.names.contains(&"Dict".to_string())));
        // assert!(!unused.iter().any(|i| i.module == "os"));

        todo!("Implement python_unused_imports test")
    }

    /// Test detecting used imports
    /// Contract: os and List are used
    #[test]
    #[ignore = "unused import detection not yet implemented"]
    fn python_used_imports() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.py", PYTHON_UNUSED_IMPORT)
            .unwrap();

        // let unused = detect_unused_imports(
        //     &test_dir.path().join("src/main.py"),
        //     Language::Python,
        // ).unwrap();
        //
        // // os and List are used
        // assert!(!unused.iter().any(|i| i.module == "os" && i.names.is_empty()));

        todo!("Implement python_used_imports test")
    }

    /// Test re-export handling
    /// Contract: Re-exported imports are not marked as unused
    #[test]
    #[ignore = "unused import detection not yet implemented"]
    fn reexport_not_unused() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file(
                "src/reexport.py",
                r#"
from utils import helper  # Re-exported

__all__ = ['helper']
"#,
            )
            .unwrap();

        // let unused = detect_unused_imports(
        //     &test_dir.path().join("src/reexport.py"),
        //     Language::Python,
        // ).unwrap();
        //
        // // helper is re-exported via __all__, not unused
        // assert!(unused.is_empty());

        todo!("Implement reexport_not_unused test")
    }

    /// Test TypeScript type-only imports
    /// Contract: type imports are not counted as unused if used in type annotations
    #[test]
    #[ignore = "unused import detection not yet implemented"]
    fn typescript_type_only_import() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/main.ts", TS_UNUSED_IMPORT).unwrap();

        // let unused = detect_unused_imports(
        //     &test_dir.path().join("src/main.ts"),
        //     Language::TypeScript,
        // ).unwrap();
        //
        // // 'unused' is unused, 'used' is used
        // // 'Config' is type-only - may or may not be flagged based on implementation
        // assert!(unused.iter().any(|i| i.names.contains(&"unused".to_string())));
        // assert!(!unused.iter().any(|i| i.names.contains(&"used".to_string())));

        todo!("Implement typescript_type_only_import test")
    }

    /// Test no unused imports case
    /// Contract: Returns empty list when all imports are used
    #[test]
    #[ignore = "unused import detection not yet implemented"]
    fn no_unused_imports() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file(
                "src/main.py",
                r#"
import os

def get_path():
    return os.getcwd()
"#,
            )
            .unwrap();

        // let unused = detect_unused_imports(
        //     &test_dir.path().join("src/main.py"),
        //     Language::Python,
        // ).unwrap();
        //
        // assert!(unused.is_empty());

        todo!("Implement no_unused_imports test")
    }
}

// =============================================================================
// Dependency Visualization (DOT) Tests
// =============================================================================

#[cfg(test)]
mod visualization_tests {
    use super::fixtures::*;
    use crate::analysis::deps::{
        analyze_dependencies, format_deps_dot, format_deps_text,
    };

    /// Test valid DOT format output
    /// Contract: Output starts with "digraph", has nodes and edges
    #[test]
    fn test_dot_format_valid() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_IMPORTS_UTILS)
            .unwrap();
        test_dir
            .add_file("src/utils.py", PYTHON_UTILS_NO_DEPS)
            .unwrap();
        test_dir
            .add_file("src/db.py", "def query(sql): pass")
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();
        let dot = format_deps_dot(&report);

        // Verify DOT structure per spec (S7-R42: quotes required)
        assert!(
            dot.starts_with("digraph deps {"),
            "Should start with digraph deps"
        );
        assert!(dot.contains("rankdir=LR"), "Should have LR layout");
        assert!(dot.contains("node [shape=box"), "Should have box nodes");
        assert!(dot.contains("->"), "Should have edges");
        assert!(dot.contains("auth.py"), "Should contain auth.py node");
        assert!(dot.contains("utils.py"), "Should contain utils.py node");
        assert!(dot.ends_with("}\n"), "Should end with closing brace");
    }

    /// Test DOT node labels
    /// Contract: Nodes have readable labels (basename, not full path)
    #[test]
    fn test_dot_node_labels() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/deeply/nested/auth.py", PYTHON_AUTH_IMPORTS_UTILS)
            .unwrap();
        test_dir
            .add_file("src/deeply/nested/utils.py", PYTHON_UTILS_NO_DEPS)
            .unwrap();
        test_dir
            .add_file("src/deeply/nested/db.py", "def query(sql): pass")
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();
        let dot = format_deps_dot(&report);

        // Should have readable labels (basename) per spec
        assert!(
            dot.contains("label=\"auth.py\"") || dot.contains("[label=\"auth.py\"]"),
            "Should have readable label for auth.py. Got:\n{}",
            dot
        );
        assert!(
            dot.contains("label=\"utils.py\"") || dot.contains("[label=\"utils.py\"]"),
            "Should have readable label for utils.py. Got:\n{}",
            dot
        );
    }

    /// Test DOT edge attributes
    /// Contract: Edges have proper DOT syntax
    #[test]
    fn test_dot_edge_syntax() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/a.py", "from b import x\ndef fa(): pass")
            .unwrap();
        test_dir.add_file("src/b.py", "def x(): pass").unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();
        let dot = format_deps_dot(&report);

        // Edges should have proper DOT syntax with quotes (S7-R42)
        assert!(dot.contains("->"), "Should have edge arrows");
        // Check that edge syntax is valid: "node1" -> "node2"
        assert!(
            dot.contains("\" -> \""),
            "Edges should use quoted identifiers. Got:\n{}",
            dot
        );
    }

    /// Test cycle highlighting in DOT
    /// Contract: Edges in cycles are highlighted with color=red (S7-R42)
    #[test]
    fn test_dot_cycle_highlighting() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/cycle_a.py", PYTHON_CYCLE_A).unwrap();
        test_dir.add_file("src/cycle_b.py", PYTHON_CYCLE_B).unwrap();
        test_dir
            .add_file("src/no_cycle.py", PYTHON_UTILS_NO_DEPS)
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Verify cycles were detected first
        assert!(
            report.stats.cycles_found >= 1,
            "Should detect at least 1 cycle. Found: {}. Deps: {:?}",
            report.stats.cycles_found,
            report.circular_dependencies
        );

        let dot = format_deps_dot(&report);

        // Cycle edges should be highlighted in red per spec
        assert!(
            dot.contains("color=red") || dot.contains("[color=red"),
            "Cycle edges should be highlighted with color=red. Got:\n{}",
            dot
        );
        assert!(
            dot.contains("penwidth=2"),
            "Cycle edges should have penwidth=2. Got:\n{}",
            dot
        );
    }

    /// Test DOT layout attributes
    /// Contract: Graph has rankdir=LR for left-to-right layout
    #[test]
    fn test_dot_layout() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_IMPORTS_UTILS)
            .unwrap();
        test_dir
            .add_file("src/utils.py", PYTHON_UTILS_NO_DEPS)
            .unwrap();
        test_dir
            .add_file("src/db.py", "def query(sql): pass")
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();
        let dot = format_deps_dot(&report);

        // Should have LR layout per spec
        assert!(
            dot.contains("rankdir=LR"),
            "Should have rankdir=LR. Got:\n{}",
            dot
        );
    }

    /// Test text format output matches spec
    /// Contract: Text output follows spec format
    #[test]
    fn test_text_format_spec_compliant() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_IMPORTS_UTILS)
            .unwrap();
        test_dir
            .add_file("src/utils.py", PYTHON_UTILS_NO_DEPS)
            .unwrap();
        test_dir
            .add_file("src/db.py", PYTHON_DB_IMPORTS_UTILS)
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();
        let text = format_deps_text(&report);

        // Per spec section 1.6:
        // "Dependency Analysis: src/"
        assert!(
            text.contains("Dependency Analysis:"),
            "Should start with 'Dependency Analysis:'"
        );

        // "Language: Python"
        assert!(
            text.contains("Language: Python"),
            "Should have 'Language: Python' (capitalized). Got:\n{}",
            text
        );

        // "Internal Dependencies (X edges, Y files):"
        assert!(
            text.contains("Internal Dependencies ("),
            "Should have 'Internal Dependencies'. Got:\n{}",
            text
        );
        assert!(
            text.contains("edges,") && text.contains("files)"),
            "Should have '(X edges, Y files)' format. Got:\n{}",
            text
        );

        // Should have indented deps: "  src/auth.py" then "    -> src/utils.py"
        assert!(
            text.contains("  src/") || text.contains("  auth.py"),
            "Should have 2-space indented file entries. Got:\n{}",
            text
        );
        assert!(
            text.contains("    ->"),
            "Should have 4-space indented deps with arrow. Got:\n{}",
            text
        );
    }

    /// Test text format with cycles
    /// Contract: Cycles are shown with [CYCLE] prefix
    #[test]
    fn test_text_format_cycles() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/cycle_a.py", PYTHON_CYCLE_A).unwrap();
        test_dir.add_file("src/cycle_b.py", PYTHON_CYCLE_B).unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();
        let text = format_deps_text(&report);

        // Per spec: "Circular Dependencies Found: N"
        assert!(
            text.contains("Circular Dependencies Found:"),
            "Should have 'Circular Dependencies Found:' header. Got:\n{}",
            text
        );

        // Per spec: "  [CYCLE] src/a.py -> src/b.py -> src/c.py -> src/a.py"
        assert!(
            text.contains("[CYCLE]"),
            "Should have '[CYCLE]' prefix. Got:\n{}",
            text
        );

        // Should show cycle loop back to start
        assert!(
            text.contains("cycle_a.py") && text.contains("cycle_b.py"),
            "Should show cycle nodes. Got:\n{}",
            text
        );
    }

    /// Test text format Stats section
    /// Contract: Stats section shows max depth, leaf files, root files
    #[test]
    fn test_text_format_stats() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_IMPORTS_UTILS)
            .unwrap();
        test_dir
            .add_file("src/utils.py", PYTHON_UTILS_NO_DEPS)
            .unwrap();
        test_dir
            .add_file("src/db.py", PYTHON_DB_IMPORTS_UTILS)
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("python".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();
        let text = format_deps_text(&report);

        // Per spec:
        // "Stats:"
        // "  Max depth: 4"
        // "  Leaf files: 3 (no outgoing deps)"
        // "  Root files: 2 (no incoming deps)"
        assert!(
            text.contains("Stats:"),
            "Should have 'Stats:' section. Got:\n{}",
            text
        );
        assert!(
            text.contains("Max depth:"),
            "Should have 'Max depth:'. Got:\n{}",
            text
        );
        assert!(
            text.contains("Leaf files:") && text.contains("(no outgoing deps)"),
            "Should have 'Leaf files: N (no outgoing deps)'. Got:\n{}",
            text
        );
        assert!(
            text.contains("Root files:") && text.contains("(no incoming deps)"),
            "Should have 'Root files: N (no incoming deps)'. Got:\n{}",
            text
        );
    }
}

// =============================================================================
// Go Dependency Resolution Tests (P0 fix)
// =============================================================================

#[cfg(test)]
mod go_deps_tests {
    use super::fixtures::*;
    use crate::analysis::deps::analyze_dependencies;
    use std::path::PathBuf;

    // -------------------------------------------------------------------------
    // Go fixtures for cross-package resolution
    // -------------------------------------------------------------------------

    const GO_MOD_MYPROJECT: &str = "module myproject\n\ngo 1.21\n";

    const GO_ROOT_MAIN: &str = r#"
package main

import (
    "fmt"
    "myproject/utils"
    "myproject/db"
)

func main() {
    fmt.Println("hello")
    utils.FormatDate(0)
    db.Query("SELECT 1")
}
"#;

    const GO_UTILS_HELPERS: &str = r#"
package utils

import "time"

func FormatDate(timestamp int64) string {
    return time.Unix(timestamp, 0).Format(time.RFC3339)
}
"#;

    const GO_UTILS_EXTRA: &str = r#"
package utils

func ExtraHelper() string {
    return "extra"
}
"#;

    const GO_DB_STORE: &str = r#"
package db

import "myproject/utils"

func Query(sql string) interface{} {
    utils.FormatDate(0)
    return nil
}
"#;

    /// Test Go cross-package imports using module path from go.mod.
    ///
    /// Given go.mod says `module myproject`, an import of `"myproject/utils"`
    /// should resolve to the `utils/` directory files in the project.
    #[test]
    fn go_cross_package_imports_via_module_path() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("go.mod", GO_MOD_MYPROJECT).unwrap();
        test_dir.add_file("main.go", GO_ROOT_MAIN).unwrap();
        test_dir
            .add_file("utils/utils.go", GO_UTILS_HELPERS)
            .unwrap();
        test_dir.add_file("db/db.go", GO_DB_STORE).unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("go".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // main.go imports myproject/utils and myproject/db
        let main_deps = report.internal_dependencies.get(&PathBuf::from("main.go"));
        assert!(
            main_deps.is_some(),
            "main.go should be in report. Keys: {:?}",
            report.internal_dependencies.keys().collect::<Vec<_>>()
        );
        let main_deps = main_deps.unwrap();
        assert!(
            main_deps
                .iter()
                .any(|p| p.to_string_lossy().contains("utils")),
            "main.go should depend on utils package. Got: {:?}",
            main_deps
        );
        assert!(
            main_deps.iter().any(|p| p.to_string_lossy().contains("db")),
            "main.go should depend on db package. Got: {:?}",
            main_deps
        );

        // db/db.go imports myproject/utils
        let db_deps = report.internal_dependencies.get(&PathBuf::from("db/db.go"));
        assert!(
            db_deps.is_some(),
            "db/db.go should be in report. Keys: {:?}",
            report.internal_dependencies.keys().collect::<Vec<_>>()
        );
        let db_deps = db_deps.unwrap();
        assert!(
            db_deps
                .iter()
                .any(|p| p.to_string_lossy().contains("utils")),
            "db/db.go should depend on utils package. Got: {:?}",
            db_deps
        );

        // Should have at least 3 internal dep edges
        assert!(
            report.stats.total_internal_deps >= 3,
            "Expected at least 3 internal deps (main->utils, main->db, db->utils). Got: {}",
            report.stats.total_internal_deps
        );
    }

    /// Test Go same-package files see each other implicitly.
    ///
    /// Multiple .go files in the same directory are in the same Go package.
    /// They don't need import statements to access each other's symbols.
    /// The dependency graph should show same-package files as mutually dependent.
    #[test]
    fn go_same_package_implicit_deps() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("go.mod", GO_MOD_MYPROJECT).unwrap();
        test_dir
            .add_file("utils/utils.go", GO_UTILS_HELPERS)
            .unwrap();
        test_dir.add_file("utils/extra.go", GO_UTILS_EXTRA).unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("go".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Both files should be in the report
        assert_eq!(
            report.stats.total_files, 2,
            "Expected 2 files in utils package"
        );

        // utils/utils.go should depend on utils/extra.go (same package)
        let utils_deps = report
            .internal_dependencies
            .get(&PathBuf::from("utils/utils.go"));
        let extra_deps = report
            .internal_dependencies
            .get(&PathBuf::from("utils/extra.go"));

        // At least one direction of same-package dependency should exist
        let has_same_pkg_dep = utils_deps
            .map(|d| d.iter().any(|p| p.to_string_lossy().contains("extra")))
            .unwrap_or(false)
            || extra_deps
                .map(|d| d.iter().any(|p| p.to_string_lossy().contains("utils.go")))
                .unwrap_or(false);

        assert!(
            has_same_pkg_dep,
            "Same-package Go files should have implicit dependencies on each other. \
             utils/utils.go deps: {:?}, utils/extra.go deps: {:?}",
            utils_deps, extra_deps
        );
    }

    /// Test Go module index uses go.mod module path for indexing.
    ///
    /// The build_module_index should read go.mod and create entries
    /// like "myproject/utils" -> utils/utils.go, not just "utils" -> utils/utils.go.
    #[test]
    fn go_module_index_includes_full_module_path() {
        use crate::analysis::deps::build_module_index;
        use crate::types::Language;

        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("go.mod", GO_MOD_MYPROJECT).unwrap();
        let utils_path = test_dir
            .add_file("utils/utils.go", GO_UTILS_HELPERS)
            .unwrap();
        let db_path = test_dir.add_file("db/db.go", GO_DB_STORE).unwrap();

        let root = dunce::canonicalize(test_dir.path()).unwrap();
        let files = vec![
            dunce::canonicalize(&utils_path).unwrap(),
            dunce::canonicalize(&db_path).unwrap(),
        ];

        let index = build_module_index(&root, &files, Language::Go);

        // Index should contain full module path entries
        assert!(
            index.contains_key("myproject/utils"),
            "Index should contain 'myproject/utils'. Keys: {:?}",
            index.keys().collect::<Vec<_>>()
        );
        assert!(
            index.contains_key("myproject/db"),
            "Index should contain 'myproject/db'. Keys: {:?}",
            index.keys().collect::<Vec<_>>()
        );

        // Should also contain relative-only path for fallback
        assert!(
            index.contains_key("utils"),
            "Index should contain 'utils' as fallback. Keys: {:?}",
            index.keys().collect::<Vec<_>>()
        );
        assert!(
            index.contains_key("db"),
            "Index should contain 'db' as fallback. Keys: {:?}",
            index.keys().collect::<Vec<_>>()
        );
    }

    /// Test Go resolve_go_import strips module prefix correctly.
    ///
    /// An import of "myproject/utils" should be resolved by:
    /// 1. Trying full path "myproject/utils" in the index (direct hit)
    /// 2. Falling back to stripping module prefix and trying "utils"
    #[test]
    fn go_resolve_import_strips_module_prefix() {
        use crate::analysis::deps::build_module_index;
        use crate::types::Language;

        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("go.mod", GO_MOD_MYPROJECT).unwrap();
        let utils_path = test_dir
            .add_file("utils/utils.go", GO_UTILS_HELPERS)
            .unwrap();

        let root = dunce::canonicalize(test_dir.path()).unwrap();
        let files = vec![dunce::canonicalize(&utils_path).unwrap()];

        let index = build_module_index(&root, &files, Language::Go);

        // Simulating what resolve_go_import does:
        // Import "myproject/utils" should find the utils file
        let result = index.get("myproject/utils");
        assert!(
            result.is_some(),
            "Should resolve 'myproject/utils' from index. Keys: {:?}",
            index.keys().collect::<Vec<_>>()
        );
    }

    /// Test real-world Go project: cobra with doc/ subdirectory.
    ///
    /// The doc/ package files import "github.com/example/mylib" which is the
    /// root-level package. This should resolve to root-level .go files.
    #[test]
    fn go_cobra_style_cross_package_imports() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("go.mod", "module github.com/example/mylib\n\ngo 1.21\n")
            .unwrap();
        test_dir
            .add_file(
                "command.go",
                r#"
package mylib

func NewCommand() string {
    return "cmd"
}
"#,
            )
            .unwrap();
        test_dir
            .add_file(
                "doc/gen.go",
                r#"
package doc

import "github.com/example/mylib"

func GenDocs() string {
    return mylib.NewCommand()
}
"#,
            )
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("go".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // doc/gen.go imports "github.com/example/mylib" which is the root package
        let doc_deps = report
            .internal_dependencies
            .get(&PathBuf::from("doc/gen.go"));
        assert!(
            doc_deps.is_some(),
            "doc/gen.go should be in report. Keys: {:?}",
            report.internal_dependencies.keys().collect::<Vec<_>>()
        );
        let doc_deps = doc_deps.unwrap();
        assert!(
            doc_deps
                .iter()
                .any(|p| p.to_string_lossy().contains("command.go")),
            "doc/gen.go should depend on root-level command.go. Got: {:?}. Full report: {:?}",
            doc_deps,
            report.internal_dependencies
        );
    }

    /// Test Go stdlib imports are not classified as internal.
    ///
    /// Stdlib imports like "fmt", "os", "net/http" should not create
    /// internal dependency edges.
    #[test]
    fn go_stdlib_not_internal() {
        use crate::analysis::deps::is_go_stdlib;

        assert!(is_go_stdlib("fmt"), "fmt should be stdlib");
        assert!(is_go_stdlib("os"), "os should be stdlib");
        assert!(is_go_stdlib("net/http"), "net/http should be stdlib");
        assert!(
            is_go_stdlib("encoding/json"),
            "encoding/json should be stdlib"
        );
        assert!(is_go_stdlib("time"), "time should be stdlib");

        assert!(
            !is_go_stdlib("github.com/spf13/cobra"),
            "cobra should NOT be stdlib"
        );
        assert!(
            !is_go_stdlib("myproject/utils"),
            "myproject/utils should NOT be stdlib"
        );
    }
}

// =============================================================================
// Edge Cases and Error Handling Tests
// =============================================================================

#[cfg(test)]
mod edge_case_tests {
    use super::fixtures::*;

    /// Test handling of dynamic imports
    /// Contract: Dynamic imports are skipped (static analysis only)
    #[test]
    #[ignore = "deps module not yet implemented"]
    fn dynamic_imports_skipped() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file(
                "src/main.py",
                r#"
import importlib

def load_module(name):
    return importlib.import_module(name)
"#,
            )
            .unwrap();

        // let report = analyze_dependencies(
        //     test_dir.path(),
        //     Language::Python,
        //     &DepsConfig::default(),
        // ).unwrap();
        //
        // // importlib itself is detected, but dynamic imports are not resolved
        // // No error should occur

        todo!("Implement dynamic_imports_skipped test")
    }

    /// Test aliased imports resolution
    /// Contract: Aliases are resolved to original module
    #[test]
    #[ignore = "deps module not yet implemented"]
    fn aliased_imports() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file(
                "src/main.py",
                r#"
import pandas as pd
import numpy as np

def process():
    pd.DataFrame()
    np.array([1, 2, 3])
"#,
            )
            .unwrap();

        // let config = DepsConfig {
        //     include_external: true,
        //     ..Default::default()
        // };
        // let report = analyze_dependencies(
        //     test_dir.path(),
        //     Language::Python,
        //     &config,
        // ).unwrap();
        //
        // // Should report pandas and numpy, not pd and np
        // assert!(report.external_dependencies.contains_key("pandas"));
        // assert!(report.external_dependencies.contains_key("numpy"));

        todo!("Implement aliased_imports test")
    }

    /// Test very deep dependency chain performance
    /// Contract: Completes within performance requirements
    #[test]
    #[ignore = "performance test"]
    fn deep_chain_performance() {
        let test_dir = TestDir::new().unwrap();

        // Create a 50-node chain
        for i in 0..50 {
            let content = if i < 49 {
                format!(
                    "from chain_{} import func\n\ndef func(): return func()",
                    i + 1
                )
            } else {
                "def func(): return 42".to_string()
            };
            test_dir
                .add_file(&format!("src/chain_{}.py", i), &content)
                .unwrap();
        }

        // let start = std::time::Instant::now();
        // let report = analyze_dependencies(
        //     test_dir.path(),
        //     Language::Python,
        //     &DepsConfig::default(),
        // ).unwrap();
        // let elapsed = start.elapsed();
        //
        // assert!(elapsed.as_millis() < 5000);  // < 5 seconds
        // assert_eq!(report.stats.total_files, 50);

        todo!("Implement deep_chain_performance test")
    }

    /// Test unsupported language error
    /// Contract: Returns appropriate error for unsupported languages
    #[test]
    #[ignore = "deps module not yet implemented"]
    fn unsupported_language_error() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/main.rb", "puts 'hello'").unwrap();

        // let result = analyze_dependencies(
        //     test_dir.path(),
        //     Language::Ruby,  // Not supported
        //     &DepsConfig::default(),
        // );
        //
        // assert!(result.is_err());

        todo!("Implement unsupported_language_error test")
    }

    /// Test path not found error
    /// Contract: Returns error when path doesn't exist
    #[test]
    #[ignore = "deps module not yet implemented"]
    fn path_not_found_error() {
        // let result = analyze_dependencies(
        //     Path::new("/nonexistent/path"),
        //     Language::Python,
        //     &DepsConfig::default(),
        // );
        //
        // assert!(result.is_err());

        todo!("Implement path_not_found_error test")
    }
}

// =============================================================================
// Java Dependency Resolution Tests (P0 fix)
// =============================================================================

#[cfg(test)]
mod java_deps_tests {
    use super::fixtures::*;
    use crate::analysis::deps::analyze_dependencies;
    use std::collections::HashMap;
    use std::path::PathBuf;

    // -------------------------------------------------------------------------
    // Unit tests: build_module_index for Java
    // -------------------------------------------------------------------------

    /// Test that build_module_index creates correct index entries for Java files.
    /// Contract: Qualified class name maps to file path, simple class name also indexed.
    #[test]
    fn test_java_module_index_qualified_name() {
        use crate::analysis::deps::build_module_index;
        use crate::types::Language;

        let test_dir = TestDir::new().unwrap();
        let prec_path = test_dir
            .add_file(
                "com/google/common/base/Preconditions.java",
                JAVA_PRECONDITIONS,
            )
            .unwrap();

        let root = test_dir.path();
        let files = vec![prec_path.clone()];
        let index = build_module_index(root, &files, Language::Java);

        // Full qualified name should be in index
        assert!(
            index.contains_key("com.google.common.base.Preconditions"),
            "Index should contain qualified name 'com.google.common.base.Preconditions'. Keys: {:?}",
            index.keys().collect::<Vec<_>>()
        );

        // Simple class name should also be in index
        assert!(
            index.contains_key("Preconditions"),
            "Index should contain simple class name 'Preconditions'. Keys: {:?}",
            index.keys().collect::<Vec<_>>()
        );
    }

    /// Test that build_module_index indexes multiple Java files in a package.
    #[test]
    fn test_java_module_index_multiple_files() {
        use crate::analysis::deps::build_module_index;
        use crate::types::Language;

        let test_dir = TestDir::new().unwrap();
        let prec_path = test_dir
            .add_file(
                "com/google/common/base/Preconditions.java",
                JAVA_PRECONDITIONS,
            )
            .unwrap();
        let strings_path = test_dir
            .add_file("com/google/common/base/Strings.java", JAVA_STRINGS)
            .unwrap();

        let root = test_dir.path();
        let files = vec![prec_path, strings_path.clone()];
        let index = build_module_index(root, &files, Language::Java);

        // Both classes should be indexed
        assert!(
            index.contains_key("com.google.common.base.Preconditions"),
            "Index should contain Preconditions"
        );
        assert!(
            index.contains_key("com.google.common.base.Strings"),
            "Index should contain Strings. Keys: {:?}",
            index.keys().collect::<Vec<_>>()
        );
        assert!(
            index.contains_key("Strings"),
            "Index should contain simple name Strings"
        );
    }

    // -------------------------------------------------------------------------
    // Unit tests: resolve_java_import
    // -------------------------------------------------------------------------

    /// Test direct resolution of fully qualified Java import.
    /// Contract: "com.google.common.base.Preconditions" resolves to Preconditions.java
    #[test]
    fn test_resolve_java_import_qualified() {
        use crate::analysis::deps::resolve_java_import;

        let root = PathBuf::from("/project");
        let current_file = PathBuf::from("/project/com/google/common/base/Joiner.java");
        let prec_path = PathBuf::from("/project/com/google/common/base/Preconditions.java");

        let mut index = HashMap::new();
        index.insert(
            "com.google.common.base.Preconditions".to_string(),
            prec_path.clone(),
        );
        index.insert("Preconditions".to_string(), prec_path.clone());

        let result = resolve_java_import(
            "com.google.common.base.Preconditions",
            &root,
            &current_file,
            &index,
        );
        assert_eq!(
            result,
            Some(prec_path),
            "Should resolve qualified import to file path"
        );
    }

    /// Test wildcard import resolution.
    /// Contract: "com.google.common.base.*" resolves to files in that package.
    #[test]
    fn test_resolve_java_import_wildcard() {
        use crate::analysis::deps::resolve_java_import;

        let root = PathBuf::from("/project");
        let current_file = PathBuf::from("/project/com/google/common/collect/Lists.java");
        let prec_path = PathBuf::from("/project/com/google/common/base/Preconditions.java");
        let strings_path = PathBuf::from("/project/com/google/common/base/Strings.java");

        let mut index = HashMap::new();
        index.insert(
            "com.google.common.base.Preconditions".to_string(),
            prec_path.clone(),
        );
        index.insert(
            "com.google.common.base.Strings".to_string(),
            strings_path.clone(),
        );

        // Wildcard import should resolve to at least one file in the package
        let result = resolve_java_import("com.google.common.base.*", &root, &current_file, &index);
        assert!(
            result.is_some(),
            "Wildcard import should resolve to at least one file in the package"
        );
    }

    /// Test static import resolution.
    /// Contract: "com.google.common.base.Preconditions.checkNotNull" (static) resolves
    /// by stripping the method name and resolving the class.
    #[test]
    fn test_resolve_java_import_static() {
        use crate::analysis::deps::resolve_java_import;

        let root = PathBuf::from("/project");
        let current_file = PathBuf::from("/project/com/google/common/collect/Sets.java");
        let prec_path = PathBuf::from("/project/com/google/common/base/Preconditions.java");

        let mut index = HashMap::new();
        index.insert(
            "com.google.common.base.Preconditions".to_string(),
            prec_path.clone(),
        );
        index.insert("Preconditions".to_string(), prec_path.clone());

        // Static import: module is "com.google.common.base.Preconditions.checkNotNull"
        // The function should strip the method name and resolve the class
        let result = resolve_java_import(
            "com.google.common.base.Preconditions.checkNotNull",
            &root,
            &current_file,
            &index,
        );
        assert_eq!(
            result,
            Some(prec_path),
            "Static import should resolve by stripping method name"
        );
    }

    /// Test that JDK imports do not resolve (they are not in the project).
    #[test]
    fn test_resolve_java_import_jdk_returns_none() {
        use crate::analysis::deps::resolve_java_import;

        let root = PathBuf::from("/project");
        let current_file = PathBuf::from("/project/com/example/App.java");
        let index = HashMap::new();

        let result = resolve_java_import("java.util.List", &root, &current_file, &index);
        assert_eq!(result, None, "JDK import should not resolve");

        let result = resolve_java_import("javax.annotation.Nullable", &root, &current_file, &index);
        assert_eq!(result, None, "javax import should not resolve");
    }

    // -------------------------------------------------------------------------
    // Integration tests: analyze_dependencies for Java
    // -------------------------------------------------------------------------

    /// Test end-to-end Java dependency analysis with internal deps.
    /// Contract: analyze_dependencies on a Java test directory returns edges.
    #[test]
    fn test_java_analyze_dependencies_basic() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file(
                "com/google/common/base/Preconditions.java",
                JAVA_PRECONDITIONS,
            )
            .unwrap();
        test_dir
            .add_file("com/google/common/base/Strings.java", JAVA_STRINGS)
            .unwrap();
        test_dir
            .add_file("com/google/common/base/Joiner.java", JAVA_JOINER)
            .unwrap();
        test_dir
            .add_file("com/google/common/base/Splitter.java", JAVA_SPLITTER)
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("java".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Should find all 4 files
        assert_eq!(report.stats.total_files, 4, "Expected 4 Java files");

        // Should have internal dependency edges
        assert!(
            report.stats.total_internal_deps > 0,
            "Expected internal dependency edges for Java, got 0. Report: {:?}",
            report.internal_dependencies
        );

        // Joiner.java imports Preconditions
        let joiner_deps = report
            .internal_dependencies
            .iter()
            .find(|(k, _)| k.to_string_lossy().contains("Joiner.java"));
        assert!(joiner_deps.is_some(), "Joiner.java should be in report");
        let (_, joiner_deps) = joiner_deps.unwrap();
        assert!(
            joiner_deps
                .iter()
                .any(|p| p.to_string_lossy().contains("Preconditions.java")),
            "Joiner.java should depend on Preconditions.java. Got: {:?}",
            joiner_deps
        );

        // Splitter.java imports both Preconditions and Strings
        let splitter_deps = report
            .internal_dependencies
            .iter()
            .find(|(k, _)| k.to_string_lossy().contains("Splitter.java"));
        assert!(splitter_deps.is_some(), "Splitter.java should be in report");
        let (_, splitter_deps) = splitter_deps.unwrap();
        assert!(
            splitter_deps
                .iter()
                .any(|p| p.to_string_lossy().contains("Preconditions.java")),
            "Splitter.java should depend on Preconditions.java. Got: {:?}",
            splitter_deps
        );
        assert!(
            splitter_deps
                .iter()
                .any(|p| p.to_string_lossy().contains("Strings.java")),
            "Splitter.java should depend on Strings.java. Got: {:?}",
            splitter_deps
        );
    }

    /// Test Java wildcard import creates dependency edges in analyze_dependencies.
    #[test]
    fn test_java_analyze_dependencies_wildcard() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file(
                "com/google/common/base/Preconditions.java",
                JAVA_PRECONDITIONS,
            )
            .unwrap();
        test_dir
            .add_file("com/google/common/base/Strings.java", JAVA_STRINGS)
            .unwrap();
        test_dir
            .add_file("com/google/common/collect/Lists.java", JAVA_WILDCARD_IMPORT)
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("java".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Lists.java uses wildcard import from base package
        // It should resolve to at least one file in com.google.common.base
        let lists_deps = report
            .internal_dependencies
            .iter()
            .find(|(k, _)| k.to_string_lossy().contains("Lists.java"));
        assert!(lists_deps.is_some(), "Lists.java should be in report");
        let (_, lists_deps) = lists_deps.unwrap();
        assert!(
            !lists_deps.is_empty(),
            "Lists.java (wildcard import) should have at least one internal dep. Got empty."
        );
    }

    /// Test Java static import creates dependency edges.
    #[test]
    fn test_java_analyze_dependencies_static_import() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file(
                "com/google/common/base/Preconditions.java",
                JAVA_PRECONDITIONS,
            )
            .unwrap();
        test_dir
            .add_file("com/google/common/collect/Sets.java", JAVA_STATIC_IMPORT)
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("java".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // Sets.java uses static import from Preconditions
        let sets_deps = report
            .internal_dependencies
            .iter()
            .find(|(k, _)| k.to_string_lossy().contains("Sets.java"));
        assert!(sets_deps.is_some(), "Sets.java should be in report");
        let (_, sets_deps) = sets_deps.unwrap();
        assert!(
            sets_deps
                .iter()
                .any(|p| p.to_string_lossy().contains("Preconditions.java")),
            "Sets.java (static import) should depend on Preconditions.java. Got: {:?}",
            sets_deps
        );
    }

    /// Test Java stdlib/JDK imports are classified as external.
    #[test]
    fn test_java_external_classification() {
        use crate::analysis::deps::is_java_stdlib;

        // JDK packages should be classified as stdlib
        assert!(
            is_java_stdlib("java.util.List"),
            "java.util.List should be stdlib"
        );
        assert!(
            is_java_stdlib("java.io.File"),
            "java.io.File should be stdlib"
        );
        assert!(
            is_java_stdlib("javax.annotation.Nullable"),
            "javax.annotation.Nullable should be stdlib"
        );

        // Project packages should NOT be stdlib
        assert!(
            !is_java_stdlib("com.google.common.base.Preconditions"),
            "com.google.common.base.Preconditions should NOT be stdlib"
        );
        assert!(
            !is_java_stdlib("org.apache.commons.lang3.StringUtils"),
            "Third-party packages should NOT be stdlib"
        );
    }

    /// Test Java classify_import integration with analyze_dependencies.
    #[test]
    fn test_java_classify_import_external() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file(
                "com/google/common/base/Preconditions.java",
                JAVA_PRECONDITIONS,
            )
            .unwrap();
        test_dir
            .add_file("com/example/app/App.java", JAVA_EXTERNAL_IMPORTS)
            .unwrap();

        let mut options = crate::analysis::deps::DepsOptions { language: Some("java".to_string()), ..Default::default() };
        options.include_external = true;

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        // App.java imports java.util.List (stdlib), javax.annotation.Nullable (stdlib/external),
        // and com.google.common.base.Preconditions (internal)
        let app_path = report
            .internal_dependencies
            .keys()
            .find(|k| k.to_string_lossy().contains("App.java"));
        assert!(app_path.is_some(), "App.java should be in report");

        // java.util and javax should NOT appear in internal deps
        let app_deps = report.internal_dependencies.get(app_path.unwrap()).unwrap();
        assert!(
            !app_deps
                .iter()
                .any(|p| p.to_string_lossy().contains("java/util")),
            "JDK imports should not be in internal deps"
        );
    }

    /// Test Java file with no imports has no dependencies.
    #[test]
    fn test_java_no_deps() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("com/example/util/Constants.java", JAVA_NO_DEPS)
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("java".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        assert_eq!(report.stats.total_files, 1, "Expected 1 file");
        assert_eq!(
            report.stats.total_internal_deps, 0,
            "Java file with no imports should have 0 internal deps"
        );
    }

    /// Test Java cycle detection works.
    #[test]
    fn test_java_cycle_detection() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("com/example/cycle/CycleA.java", JAVA_CYCLE_A)
            .unwrap();
        test_dir
            .add_file("com/example/cycle/CycleB.java", JAVA_CYCLE_B)
            .unwrap();

        let options = crate::analysis::deps::DepsOptions { language: Some("java".to_string()), ..Default::default() };

        let report = analyze_dependencies(test_dir.path(), &options).unwrap();

        assert_eq!(
            report.stats.cycles_found, 1,
            "Expected 1 cycle in Java. Found: {}. Cycles: {:?}",
            report.stats.cycles_found, report.circular_dependencies
        );
    }
}
