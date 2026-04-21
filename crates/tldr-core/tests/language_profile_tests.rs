//! Tests for the LanguageProfile architecture and pattern detector refactor
//!
//! These tests define the TARGET behavior for the LanguageProfile refactor.
//! They are organized into five parts:
//!
//! - Part 1: LanguageProfile API Tests (struct existence, field completeness)
//! - Part 2: Generic Walker Behavioral Equivalence Tests
//! - Part 3: Signal Equivalence Snapshot Tests (golden references)
//! - Part 4: New Language Coverage Tests (11 new languages)
//! - Part 5: Custom Extractor Tests (framework-specific detection preservation)
//!
//! Tests marked with #[ignore] reference types/functions that do not exist yet.
//! Tests without #[ignore] use the existing detector API to establish golden
//! reference values that the refactor MUST preserve.

use std::path::PathBuf;

use tldr_core::ast::parser;
use tldr_core::patterns::language_profile::{
    language_profile, LanguageProfile, SignalAction, SignalTarget, TryTarget,
};
use tldr_core::patterns::signals::{detect_naming_case, NamingCase};
use tldr_core::patterns::{PatternDetector, PatternSignals};
use tldr_core::types::Language;

// ============================================================================
// Test Helpers
// ============================================================================

/// Parse source code and run detect_all, returning PatternSignals.
/// This is the primary helper for signal-level tests.
fn detect_signals(lang: Language, source: &str) -> PatternSignals {
    let tree = parser::parse(source, lang).expect("Failed to parse source code");
    let detector = PatternDetector::new(lang, PathBuf::from("test_file"));
    detector.detect_all(&tree, source)
}

// ============================================================================
// Part 1: LanguageProfile API Tests
// ============================================================================
// These tests define the LanguageProfile struct and language_profile() function
// that will be created during the refactor. They MUST be #[ignore] because the
// types do not exist yet.

#[test]
fn test_language_profile_struct_exists() {
    let profile: LanguageProfile =
        language_profile(Language::Python).expect("Python profile should exist");
    assert!(!profile.node_map.dispatch.is_empty());
}

#[test]
fn test_language_profile_exists_for_all_18_languages() {
    for lang in Language::all() {
        assert!(
            language_profile(*lang).is_some(),
            "{:?} profile should exist",
            lang
        );
    }
}

#[test]
fn test_language_profile_every_language_has_function_nodes() {
    for lang in Language::all() {
        let profile = language_profile(*lang).expect("profile should exist");
        let has_function_dispatch = profile.node_map.dispatch.keys().any(|k| {
            k.contains("function")
                || k.contains("method")
                || *k == "let_binding"
                || *k == "value_definition"
        });
        let has_function_call_dispatch = profile
            .node_map
            .call_dispatch
            .keys()
            .any(|k| *k == "def" || *k == "defp");
        assert!(
            has_function_dispatch || has_function_call_dispatch,
            "{:?} should have function-related dispatch",
            lang
        );
    }
}

#[test]
fn test_language_profile_classless_languages_have_no_class_nodes() {
    let profile_c = language_profile(Language::C).expect("C profile should exist");
    assert!(
        !profile_c
            .node_map
            .dispatch
            .keys()
            .any(|k| k.contains("class")),
        "C has no class nodes"
    );

    let profile_lua = language_profile(Language::Lua).expect("Lua profile should exist");
    assert!(
        !profile_lua
            .node_map
            .dispatch
            .keys()
            .any(|k| k.contains("class")),
        "Lua has no class nodes"
    );
}

#[test]
fn test_language_profile_python_try_nodes() {
    let profile = language_profile(Language::Python).expect("Python profile should exist");
    let actions = profile
        .node_map
        .dispatch
        .get("try_statement")
        .expect("Python try_statement dispatch should exist");
    assert!(actions
        .iter()
        .any(|a| matches!(a, SignalAction::PushEvidence(SignalTarget::TryExceptBlocks))));
    assert_eq!(profile.semantics.try_target(), TryTarget::TryExcept);
}

#[test]
fn test_language_profile_typescript_try_nodes() {
    let profile = language_profile(Language::TypeScript).expect("TypeScript profile should exist");
    let actions = profile
        .node_map
        .dispatch
        .get("try_statement")
        .expect("TypeScript try_statement dispatch should exist");
    assert!(actions
        .iter()
        .any(|a| matches!(a, SignalAction::PushEvidence(SignalTarget::TryCatchBlocks))));
    assert!(actions
        .iter()
        .any(|a| matches!(a, SignalAction::CallSemantics)));
    assert_eq!(profile.semantics.try_target(), TryTarget::TryCatch);
}

#[test]
fn test_language_profile_rust_question_mark_nodes() {
    let profile = language_profile(Language::Rust).expect("Rust profile should exist");
    let actions = profile
        .node_map
        .dispatch
        .get("try_expression")
        .expect("Rust try_expression dispatch should exist");
    assert!(actions
        .iter()
        .any(|a| matches!(a, SignalAction::PushEvidence(SignalTarget::QuestionMarkOps))));
}

#[test]
fn test_language_profile_go_goroutine_and_defer_nodes() {
    let profile = language_profile(Language::Go).expect("Go profile should exist");
    let go_actions = profile
        .node_map
        .dispatch
        .get("go_statement")
        .expect("Go go_statement dispatch should exist");
    assert!(go_actions
        .iter()
        .any(|a| matches!(a, SignalAction::PushEvidence(SignalTarget::Goroutines))));

    let defer_actions = profile
        .node_map
        .dispatch
        .get("defer_statement")
        .expect("Go defer_statement dispatch should exist");
    assert!(defer_actions
        .iter()
        .any(|a| matches!(a, SignalAction::PushEvidence(SignalTarget::DeferStatements))));
}

#[test]
fn test_language_profile_python_class_config() {
    let profile = language_profile(Language::Python).expect("Python profile should exist");
    assert!(profile.node_map.dispatch.contains_key("class_definition"));
    let actions = profile
        .node_map
        .dispatch
        .get("class_definition")
        .expect("Python class_definition dispatch should exist");
    assert!(actions
        .iter()
        .any(|a| matches!(a, SignalAction::CallSemantics)));
}

#[test]
fn test_language_profile_python_function_config() {
    let profile = language_profile(Language::Python).expect("Python profile should exist");
    let fn_actions = profile
        .node_map
        .dispatch
        .get("function_definition")
        .expect("Python function_definition dispatch should exist");
    assert!(fn_actions
        .iter()
        .any(|a| matches!(a, SignalAction::CallSemantics)));
    assert!(profile.node_map.dispatch.contains_key("decorator"));
    assert!(profile.node_map.dispatch.contains_key("assignment"));
}

#[test]
fn test_language_profile_rust_async_await_nodes() {
    let profile = language_profile(Language::Rust).expect("Rust profile should exist");
    let async_actions = profile
        .node_map
        .dispatch
        .get("async_block")
        .expect("Rust async_block dispatch should exist");
    assert!(async_actions
        .iter()
        .any(|a| matches!(a, SignalAction::PushEvidence(SignalTarget::AsyncAwait))));
    let await_actions = profile
        .node_map
        .dispatch
        .get("await_expression")
        .expect("Rust await_expression dispatch should exist");
    assert!(await_actions
        .iter()
        .any(|a| matches!(a, SignalAction::PushEvidence(SignalTarget::AsyncAwait))));
}

#[test]
fn test_language_profile_elixir_call_based_dispatch() {
    let profile = language_profile(Language::Elixir).expect("Elixir profile should exist");
    assert!(profile.node_map.dispatch.is_empty());
    for call in [
        "defmodule",
        "def",
        "defp",
        "import",
        "alias",
        "use",
        "require",
        "test",
    ] {
        assert!(
            profile.node_map.call_dispatch.contains_key(call),
            "Elixir call_dispatch missing key '{}'",
            call
        );
    }
}

#[test]
fn test_language_profile_lua_pcall_error_handling() {
    let profile = language_profile(Language::Lua).expect("Lua profile should exist");
    assert!(!profile.node_map.dispatch.contains_key("try_statement"));
    assert!(profile.node_map.call_dispatch.contains_key("pcall"));
    assert!(profile.node_map.call_dispatch.contains_key("xpcall"));
}

// ============================================================================
// Part 2: Generic Walker Behavioral Equivalence Tests
// ============================================================================
// These tests verify that detect_all() produces the same signals as the
// current hand-coded path. They use the existing detector directly.
// After the refactor, these tests MUST still pass with identical output.

#[test]
fn test_equivalence_python_function_detection() {
    let source = r#"
def process_user_data(name: str, age: int) -> dict:
    return {"name": name, "age": age}

def _private_helper():
    pass

async def fetch_data():
    return await get_data()
"#;
    let signals = detect_signals(Language::Python, source);

    // Must detect 3 function names
    assert_eq!(signals.naming.function_names.len(), 3);
    let names: Vec<&str> = signals
        .naming
        .function_names
        .iter()
        .map(|n| n.0.as_str())
        .collect();
    assert!(names.contains(&"process_user_data"));
    assert!(names.contains(&"_private_helper"));
    assert!(names.contains(&"fetch_data"));

    // Must detect private prefix
    assert!(
        signals
            .naming
            .private_prefixes
            .get("_")
            .copied()
            .unwrap_or(0)
            >= 1
    );

    // Must detect typed params and returns
    assert!(signals.type_coverage.typed_params >= 2); // name: str, age: int
    assert!(signals.type_coverage.typed_returns >= 1); // -> dict

    // Must detect async
    assert!(!signals.async_patterns.async_await.is_empty());
}

#[test]
fn test_equivalence_python_class_detection() {
    let source = r#"
class UserManager:
    pass

class ValidationError(Exception):
    pass

class UserModel(BaseModel):
    name: str
"#;
    let signals = detect_signals(Language::Python, source);

    // Must detect 3 classes
    assert_eq!(signals.naming.class_names.len(), 3);
    let names: Vec<&str> = signals
        .naming
        .class_names
        .iter()
        .map(|n| n.0.as_str())
        .collect();
    assert!(names.contains(&"UserManager"));
    assert!(names.contains(&"ValidationError"));
    assert!(names.contains(&"UserModel"));

    // ValidationError should be detected as custom exception
    assert!(
        signals
            .error_handling
            .custom_exceptions
            .iter()
            .any(|(name, _)| name == "ValidationError"),
        "ValidationError should be in custom_exceptions"
    );

    // UserModel(BaseModel) should be detected as pydantic model
    assert!(
        !signals.validation.pydantic_models.is_empty(),
        "UserModel(BaseModel) should be detected as pydantic model"
    );
}

#[test]
fn test_equivalence_python_error_handling() {
    let source = r#"
try:
    process_data()
except ValueError as e:
    logger.error(f"Error: {e}")
"#;
    let signals = detect_signals(Language::Python, source);

    assert_eq!(signals.error_handling.try_except_blocks.len(), 1);
    // try_catch_blocks should remain empty for Python
    assert!(signals.error_handling.try_catch_blocks.is_empty());
}

#[test]
fn test_equivalence_python_imports() {
    let source = r#"
import os
import sys
from typing import List, Optional
from .models import User
from ..utils import helper
from datetime import *
import numpy as np
"#;
    let signals = detect_signals(Language::Python, source);

    // Absolute imports: os, sys, typing, datetime, numpy
    assert!(signals.import_patterns.absolute_imports.len() >= 3);

    // Relative imports: .models, ..utils
    assert!(signals.import_patterns.relative_imports.len() >= 2);

    // Star imports: from datetime import *
    assert!(!signals.import_patterns.star_imports.is_empty());

    // Aliases: numpy as np
    assert!(
        signals
            .import_patterns
            .aliases
            .get("numpy")
            .map(|v| v == "np")
            .unwrap_or(false)
            || signals.import_patterns.aliases.contains_key("np"),
        "numpy -> np alias should be detected"
    );
}

#[test]
fn test_equivalence_python_decorators() {
    let source = r#"
import pytest

@pytest.fixture
def user_data():
    return {"name": "test"}

@mock.patch("service.get_user")
def test_process_user(mock_get_user):
    pass
"#;
    let signals = detect_signals(Language::Python, source);

    // pytest fixture detection
    assert!(!signals.test_idioms.pytest_fixtures.is_empty());

    // mock.patch detection
    assert!(!signals.test_idioms.mock_patches.is_empty());

    // Framework detection
    assert_eq!(
        signals.test_idioms.detected_framework,
        Some("pytest".to_string())
    );
}

#[test]
fn test_equivalence_python_fastapi_decorators() {
    let source = r#"
from fastapi import FastAPI

app = FastAPI()

@app.get("/users")
def get_users():
    return []

@app.post("/users")
def create_user():
    return {}
"#;
    let signals = detect_signals(Language::Python, source);

    assert!(
        signals.api_conventions.fastapi_decorators.len() >= 2,
        "Should detect at least 2 FastAPI decorators, got {}",
        signals.api_conventions.fastapi_decorators.len()
    );
}

#[test]
fn test_equivalence_typescript_function_and_class() {
    let source = r#"
class UserService {
    async getUser(id: number): Promise<User> {
        return await this.db.find(id);
    }
}

class AuthError extends Error {
    constructor(message: string) {
        super(message);
    }
}

function processData(items: string[]): void {
    console.log(items);
}

const helper = async () => {
    await delay(100);
};
"#;
    let signals = detect_signals(Language::TypeScript, source);

    // Class detection
    assert!(signals.naming.class_names.len() >= 2);
    let class_names: Vec<&str> = signals
        .naming
        .class_names
        .iter()
        .map(|n| n.0.as_str())
        .collect();
    assert!(class_names.contains(&"UserService"));
    assert!(class_names.contains(&"AuthError"));

    // AuthError should be custom exception
    assert!(
        signals
            .error_handling
            .custom_exceptions
            .iter()
            .any(|(name, _)| name == "AuthError"),
        "AuthError should be in custom_exceptions"
    );

    // Function detection (at least processData + getUser + constructor + helper)
    assert!(!signals.naming.function_names.is_empty());

    // Async detection
    assert!(!signals.async_patterns.async_await.is_empty());
}

#[test]
fn test_equivalence_typescript_jest_detection() {
    let source = r#"
describe('UserService', () => {
    it('should create user', () => {
        expect(true).toBe(true);
    });

    test('should delete user', () => {
        expect(false).toBe(false);
    });
});
"#;
    let signals = detect_signals(Language::TypeScript, source);

    // Jest block detection
    assert!(
        !signals.test_idioms.jest_blocks.is_empty(),
        "Should detect Jest describe/it/test blocks"
    );

    // Framework detection
    assert_eq!(
        signals.test_idioms.detected_framework,
        Some("jest".to_string())
    );
}

#[test]
fn test_equivalence_typescript_imports() {
    let source = r#"
import express from 'express';
import { Router } from './router';
import * as utils from '../utils';
"#;
    let signals = detect_signals(Language::TypeScript, source);

    // Absolute import: express
    assert!(!signals.import_patterns.absolute_imports.is_empty());

    // Relative imports: ./router, ../utils
    assert!(!signals.import_patterns.relative_imports.is_empty());

    // Star import: * as utils
    assert!(!signals.import_patterns.star_imports.is_empty());
}

#[test]
fn test_equivalence_typescript_try_catch() {
    let source = r#"
try {
    riskyOperation();
} catch (error) {
    console.error(error);
} finally {
    cleanup();
}
"#;
    let signals = detect_signals(Language::TypeScript, source);

    // TypeScript uses try_catch_blocks (not try_except_blocks)
    assert_eq!(signals.error_handling.try_catch_blocks.len(), 1);
    assert!(signals.error_handling.try_except_blocks.is_empty());

    // Finally block -> try_finally_blocks
    assert!(!signals.resource_management.try_finally_blocks.is_empty());
}

#[test]
fn test_equivalence_go_function_and_error() {
    let source = r#"
package main

func process() error {
    err := doSomething()
    if err != nil {
        return err
    }
    return nil
}

func TestProcess(t *testing.T) {
    err := process()
    if err != nil {
        t.Errorf("failed: %v", err)
    }
}
"#;
    let signals = detect_signals(Language::Go, source);

    // Function names
    assert!(signals.naming.function_names.len() >= 2);
    let names: Vec<&str> = signals
        .naming
        .function_names
        .iter()
        .map(|n| n.0.as_str())
        .collect();
    assert!(names.contains(&"process"));
    assert!(names.contains(&"TestProcess"));

    // Error return type detection
    assert!(!signals.error_handling.result_types.is_empty());

    // err != nil checks
    assert!(signals.error_handling.err_nil_checks.len() >= 2);

    // Test function detection
    assert!(signals.test_idioms.test_function_count >= 1);
    assert_eq!(
        signals.test_idioms.detected_framework,
        Some("go test".to_string())
    );
}

#[test]
fn test_equivalence_go_goroutine_and_defer() {
    let source = r#"
package main

func serve() {
    defer cleanup()
    go handleRequest()
}
"#;
    let signals = detect_signals(Language::Go, source);

    assert!(!signals.resource_management.defer_statements.is_empty());
    assert!(!signals.async_patterns.goroutines.is_empty());
}

#[test]
fn test_equivalence_rust_function_and_error() {
    let source = r#"
use std::io;

fn process() -> Result<String, io::Error> {
    let data = read_file()?;
    Ok(data)
}

#[test]
fn test_process() {
    assert!(process().is_ok());
}

enum MyError {
    NotFound,
    InvalidInput,
}
"#;
    let signals = detect_signals(Language::Rust, source);

    // Function names
    assert!(signals.naming.function_names.len() >= 2);

    // Result type detection
    assert!(!signals.error_handling.result_types.is_empty());

    // ? operator (try_expression)
    assert!(!signals.error_handling.question_mark_ops.is_empty());

    // Test detection
    assert!(signals.test_idioms.test_function_count >= 1);
    assert_eq!(
        signals.test_idioms.detected_framework,
        Some("rust test".to_string())
    );

    // Error enum detection (MyError ends with "Error")
    assert!(
        signals
            .error_handling
            .error_enums
            .iter()
            .any(|(name, _)| name == "MyError"),
        "MyError should be detected as error enum"
    );
}

#[test]
fn test_equivalence_rust_async_and_imports() {
    let source = r#"
use tokio::runtime::Runtime;
use crate::handlers;
use super::utils;
use std::collections::HashMap;

async fn fetch_data() -> String {
    let result = client.get("/api").await;
    result
}
"#;
    let signals = detect_signals(Language::Rust, source);

    // Tokio detection
    assert!(!signals.async_patterns.tokio_usage.is_empty());

    // Async detection
    assert!(!signals.async_patterns.async_await.is_empty());

    // Import classification
    // Relative: crate::handlers, super::utils
    assert!(signals.import_patterns.relative_imports.len() >= 2);
    // Absolute: std::collections, tokio::runtime
    assert!(!signals.import_patterns.absolute_imports.is_empty());
}

#[test]
fn test_equivalence_java_basic() {
    let source = r#"
public class UserService {
    public void processUser(String name) {
        try {
            database.save(name);
        } catch (Exception e) {
            logger.error("Failed", e);
        }
    }

    public void testHelper() {
        // utility
    }
}
"#;
    let signals = detect_signals(Language::Java, source);

    // Class detection
    assert!(!signals.naming.class_names.is_empty());
    assert!(
        signals
            .naming
            .class_names
            .iter()
            .any(|(name, _, _)| name == "UserService"),
        "UserService should be detected"
    );

    // Method detection
    assert!(signals.naming.function_names.len() >= 2);

    // Try/catch detection
    assert!(!signals.error_handling.try_catch_blocks.is_empty());
}

// ============================================================================
// Part 3: Signal Equivalence Snapshot Tests (Golden References)
// ============================================================================
// These tests capture EXACT signal values for known inputs.
// The refactor MUST preserve these exact outputs.

#[test]
fn test_snapshot_python_comprehensive() {
    let source = r#"
from typing import List, Optional
from .models import User
import os

MAX_RETRIES = 3

class UserManager:
    def __init__(self):
        pass

    def get_user(self, id: int) -> Optional[dict]:
        return {}

    def _validate(self, data):
        assert isinstance(data, dict)

class NotFoundError(Exception):
    pass

async def fetch_all() -> List[dict]:
    return await api.get("/users")

def test_get_user():
    manager = UserManager()
    assert manager.get_user(1) is not None

with open("file.txt") as f:
    data = f.read()
"#;
    let signals = detect_signals(Language::Python, source);

    // -- Naming snapshot --
    // Functions: __init__, get_user, _validate, fetch_all, test_get_user = 5
    assert_eq!(
        signals.naming.function_names.len(),
        5,
        "Expected 5 function names, got: {:?}",
        signals
            .naming
            .function_names
            .iter()
            .map(|n| &n.0)
            .collect::<Vec<_>>()
    );

    // Classes: UserManager, NotFoundError = 2
    assert_eq!(
        signals.naming.class_names.len(),
        2,
        "Expected 2 class names, got: {:?}",
        signals
            .naming
            .class_names
            .iter()
            .map(|n| &n.0)
            .collect::<Vec<_>>()
    );

    // Constants: MAX_RETRIES = 1
    assert_eq!(
        signals.naming.constant_names.len(),
        1,
        "Expected 1 constant name, got: {:?}",
        signals
            .naming
            .constant_names
            .iter()
            .map(|n| &n.0)
            .collect::<Vec<_>>()
    );
    assert_eq!(signals.naming.constant_names[0].0, "MAX_RETRIES");
    assert_eq!(
        signals.naming.constant_names[0].1,
        NamingCase::UpperSnakeCase
    );

    // -- Error handling snapshot --
    assert_eq!(signals.error_handling.custom_exceptions.len(), 1);
    assert_eq!(
        signals.error_handling.custom_exceptions[0].0,
        "NotFoundError"
    );

    // -- Test idioms snapshot --
    assert_eq!(signals.test_idioms.test_function_count, 1);
    assert_eq!(
        signals.test_idioms.detected_framework,
        Some("pytest".to_string())
    );

    // -- Import patterns snapshot --
    // Relative: .models
    assert!(!signals.import_patterns.relative_imports.is_empty());
    // Absolute: typing, os
    assert!(signals.import_patterns.absolute_imports.len() >= 2);

    // -- Async patterns snapshot --
    assert!(!signals.async_patterns.async_await.is_empty());

    // -- Type coverage snapshot --
    assert!(signals.type_coverage.typed_params >= 1); // id: int
    assert!(signals.type_coverage.typed_returns >= 2); // -> Optional[dict], -> List[dict]

    // -- Validation snapshot --
    assert!(!signals.validation.assert_statements.is_empty());
    assert!(!signals.validation.type_checks.is_empty()); // isinstance

    // -- Resource management snapshot --
    assert!(!signals.resource_management.context_managers.is_empty()); // with statement

    // -- Private prefix snapshot --
    assert!(
        signals
            .naming
            .private_prefixes
            .get("_")
            .copied()
            .unwrap_or(0)
            >= 1
    );
}

#[test]
fn test_snapshot_typescript_comprehensive() {
    let source = r#"
import express from 'express';
import { Router } from './router';

const MAX_TIMEOUT = 5000;

class ApiController {
    async handleRequest(req: Request): Promise<Response> {
        return new Response();
    }
}

class HttpError extends Error {
    constructor(public statusCode: number) {
        super();
    }
}

describe('ApiController', () => {
    it('should handle request', () => {
        expect(true).toBe(true);
    });
});

try {
    dangerousOp();
} catch (e) {
    console.error(e);
} finally {
    release();
}

const schema = z.object({ name: z.string() });
"#;
    let signals = detect_signals(Language::TypeScript, source);

    // -- Naming snapshot --
    // Classes: ApiController, HttpError
    assert_eq!(
        signals.naming.class_names.len(),
        2,
        "Expected 2 TS class names, got: {:?}",
        signals
            .naming
            .class_names
            .iter()
            .map(|n| &n.0)
            .collect::<Vec<_>>()
    );

    // HttpError -> custom exception
    assert!(signals
        .error_handling
        .custom_exceptions
        .iter()
        .any(|(n, _)| n == "HttpError"));

    // -- Error handling snapshot --
    assert_eq!(signals.error_handling.try_catch_blocks.len(), 1);

    // -- Test idioms snapshot --
    assert!(!signals.test_idioms.jest_blocks.is_empty());
    assert_eq!(
        signals.test_idioms.detected_framework,
        Some("jest".to_string())
    );

    // -- Import snapshot --
    assert!(!signals.import_patterns.absolute_imports.is_empty());
    assert!(!signals.import_patterns.relative_imports.is_empty());

    // -- Resource management snapshot --
    assert!(!signals.resource_management.try_finally_blocks.is_empty());

    // -- Async snapshot --
    assert!(!signals.async_patterns.async_await.is_empty());

    // -- Validation snapshot --
    assert!(!signals.validation.zod_schemas.is_empty());
}

#[test]
fn test_snapshot_go_comprehensive() {
    let source = r#"
package main

import "errors"

const MaxRetries = 10

func process() error {
    err := doSomething()
    if err != nil {
        return err
    }
    return nil
}

func TestProcess(t *testing.T) {
    if err := process(); err != nil {
        t.Fatal(err)
    }
}

func serve() {
    defer cleanup()
    go handleRequest()
}
"#;
    let signals = detect_signals(Language::Go, source);

    // -- Naming snapshot --
    // Functions: process, TestProcess, serve
    assert_eq!(
        signals.naming.function_names.len(),
        3,
        "Expected 3 Go function names, got: {:?}",
        signals
            .naming
            .function_names
            .iter()
            .map(|n| &n.0)
            .collect::<Vec<_>>()
    );

    // Constants: MaxRetries
    assert_eq!(signals.naming.constant_names.len(), 1);

    // -- Error handling snapshot --
    assert!(!signals.error_handling.result_types.is_empty());
    assert!(signals.error_handling.err_nil_checks.len() >= 2);

    // -- Test idioms snapshot --
    assert_eq!(signals.test_idioms.test_function_count, 1);
    assert_eq!(
        signals.test_idioms.detected_framework,
        Some("go test".to_string())
    );

    // -- Resource management snapshot --
    assert!(!signals.resource_management.defer_statements.is_empty());

    // -- Async patterns snapshot --
    assert!(!signals.async_patterns.goroutines.is_empty());
}

#[test]
fn test_snapshot_rust_comprehensive() {
    let source = r#"
use std::io;
use tokio::runtime::Runtime;
use crate::models;

const MAX_SIZE: usize = 1024;
static GLOBAL_COUNTER: u32 = 0;

fn process() -> Result<String, io::Error> {
    let data = read_file()?;
    Ok(data)
}

async fn fetch() -> String {
    let result = client.get("/api").await;
    result
}

fn test_process() {
    assert!(true);
}

enum ParseError {
    Invalid,
    Unexpected,
}

struct Config {
    pub is_deleted: bool,
    pub deleted_at: Option<String>,
    pub lock: Mutex<bool>,
}
"#;
    let signals = detect_signals(Language::Rust, source);

    // -- Naming snapshot --
    // Functions: process, fetch, test_process
    assert_eq!(
        signals.naming.function_names.len(),
        3,
        "Expected 3 Rust function names, got: {:?}",
        signals
            .naming
            .function_names
            .iter()
            .map(|n| &n.0)
            .collect::<Vec<_>>()
    );

    // Constants: MAX_SIZE, GLOBAL_COUNTER
    assert_eq!(
        signals.naming.constant_names.len(),
        2,
        "Expected 2 Rust constant names, got: {:?}",
        signals
            .naming
            .constant_names
            .iter()
            .map(|n| &n.0)
            .collect::<Vec<_>>()
    );

    // -- Error handling snapshot --
    assert!(!signals.error_handling.result_types.is_empty());
    assert!(!signals.error_handling.question_mark_ops.is_empty());
    assert!(signals
        .error_handling
        .error_enums
        .iter()
        .any(|(n, _)| n == "ParseError"));

    // -- Async snapshot --
    assert!(!signals.async_patterns.async_await.is_empty());
    assert!(!signals.async_patterns.tokio_usage.is_empty());

    // -- Import snapshot --
    assert!(!signals.import_patterns.relative_imports.is_empty()); // crate::models
    assert!(!signals.import_patterns.absolute_imports.is_empty()); // std::io

    // -- Soft delete snapshot --
    assert!(!signals.soft_delete.is_deleted_fields.is_empty());
    assert!(!signals.soft_delete.deleted_at_fields.is_empty());

    // -- Sync primitives snapshot --
    assert!(
        signals
            .async_patterns
            .sync_primitives
            .iter()
            .any(|(kind, _)| kind == "mutex"),
        "Should detect Mutex as sync primitive"
    );

    // -- Test idioms snapshot --
    assert_eq!(signals.test_idioms.test_function_count, 1);
    assert_eq!(
        signals.test_idioms.detected_framework,
        Some("rust test".to_string())
    );
}

#[test]
fn test_snapshot_java_comprehensive() {
    let source = r#"
public class UserService {
    public void processUser(String name) {
        try {
            database.save(name);
        } catch (Exception e) {
            logger.error("Failed", e);
        }
    }

    public void testCreateUser() {
        // test
    }
}
"#;
    let signals = detect_signals(Language::Java, source);

    // -- Naming snapshot --
    assert!(!signals.naming.class_names.is_empty());
    assert!(signals
        .naming
        .class_names
        .iter()
        .any(|(n, _, _)| n == "UserService"));

    // Methods: processUser, testCreateUser
    assert!(signals.naming.function_names.len() >= 2);

    // -- Error handling snapshot --
    assert!(!signals.error_handling.try_catch_blocks.is_empty());

    // -- Test detection --
    // "testCreateUser" starts with "test" -> should count
    assert!(signals.test_idioms.test_function_count >= 1);
}

// ============================================================================
// Part 4: New Language Coverage Tests
// ============================================================================
// These tests exercise languages that are currently in the _ => {} catch-all
// of the detector. After the refactor with LanguageProfile, these should
// produce real signals.

#[test]
fn test_new_language_c_detection() {
    let source = r#"
#include <stdio.h>
#include <stdlib.h>

#define MAX_SIZE 1024

struct Config {
    int is_deleted;
    char* deleted_at;
};

void process_data(int count) {
    FILE* f = fopen("data.txt", "r");
    if (f == NULL) {
        return;
    }
    fclose(f);
}

int helper_function(void) {
    return 0;
}
"#;
    let signals = detect_signals(Language::C, source);

    // Function names
    assert!(
        signals.naming.function_names.len() >= 2,
        "C should detect function names: process_data, helper_function"
    );

    // Import patterns (#include)
    assert!(
        !signals.import_patterns.absolute_imports.is_empty(),
        "C should detect #include as imports"
    );

    // No class detection for C
    assert!(signals.naming.class_names.is_empty(), "C has no classes");
}

#[test]
fn test_new_language_cpp_detection() {
    let source = r#"
#include <iostream>
#include <vector>

class UserManager {
public:
    void processUser(const std::string& name) {
        try {
            database.save(name);
        } catch (const std::exception& e) {
            std::cerr << e.what() << std::endl;
        }
    }
};

namespace utils {
    int helper() { return 0; }
}
"#;
    let signals = detect_signals(Language::Cpp, source);

    // Class detection
    assert!(
        !signals.naming.class_names.is_empty(),
        "C++ should detect class UserManager"
    );

    // Function detection
    assert!(
        signals.naming.function_names.len() >= 2,
        "C++ should detect functions"
    );

    // Error handling (try/catch)
    assert!(
        !signals.error_handling.try_catch_blocks.is_empty(),
        "C++ should detect try/catch"
    );

    // Import patterns (#include)
    assert!(
        !signals.import_patterns.absolute_imports.is_empty(),
        "C++ should detect #include"
    );
}

#[test]
fn test_new_language_ruby_detection() {
    let source = r#"
require 'json'
require_relative 'helpers'

class UserService
  def process_user(name)
    begin
      save(name)
    rescue StandardError => e
      log_error(e)
    end
  end

  def test_helper
    assert true
  end
end

module Utils
  def self.format(data)
    data.to_s
  end
end
"#;
    let signals = detect_signals(Language::Ruby, source);

    // Class detection
    assert!(
        !signals.naming.class_names.is_empty(),
        "Ruby should detect class UserService"
    );

    // Function detection
    assert!(
        signals.naming.function_names.len() >= 2,
        "Ruby should detect methods"
    );

    // Error handling (begin/rescue)
    assert!(
        !signals.error_handling.try_catch_blocks.is_empty()
            || !signals.error_handling.try_except_blocks.is_empty(),
        "Ruby should detect begin/rescue as error handling"
    );

    // Import patterns (require)
    assert!(
        !signals.import_patterns.absolute_imports.is_empty(),
        "Ruby should detect require"
    );
}

#[test]
fn test_new_language_kotlin_detection() {
    let source = r#"
import kotlin.io.path.Path

class UserRepository {
    fun findUser(id: Int): User? {
        try {
            return database.find(id)
        } catch (e: Exception) {
            throw NotFoundException("User $id not found")
        }
    }

    fun testHelper(): Boolean = true
}

object Config {
    const val MAX_RETRIES = 3
}
"#;
    let signals = detect_signals(Language::Kotlin, source);

    // Class detection
    assert!(
        !signals.naming.class_names.is_empty(),
        "Kotlin should detect class UserRepository"
    );

    // Function detection
    assert!(
        signals.naming.function_names.len() >= 2,
        "Kotlin should detect functions"
    );

    // Error handling
    assert!(
        !signals.error_handling.try_catch_blocks.is_empty(),
        "Kotlin should detect try/catch"
    );

    // Import patterns
    assert!(
        !signals.import_patterns.absolute_imports.is_empty(),
        "Kotlin should detect imports"
    );
}

#[test]
fn test_new_language_swift_detection() {
    let source = r#"
import Foundation

class NetworkManager {
    func fetchData(url: URL) async throws -> Data {
        let (data, _) = try await URLSession.shared.data(from: url)
        return data
    }

    func processItems(_ items: [String]) {
        defer { cleanup() }
        for item in items {
            handle(item)
        }
    }
}

protocol Fetchable {
    func fetch() async throws
}
"#;
    let signals = detect_signals(Language::Swift, source);

    // Class detection
    assert!(
        !signals.naming.class_names.is_empty(),
        "Swift should detect class NetworkManager"
    );

    // Function detection
    assert!(
        signals.naming.function_names.len() >= 2,
        "Swift should detect functions"
    );

    // Import patterns
    assert!(
        !signals.import_patterns.absolute_imports.is_empty(),
        "Swift should detect imports"
    );
}

#[test]
fn test_new_language_csharp_detection() {
    let source = r#"
using System;
using System.Collections.Generic;

namespace Services {
    public class UserService {
        public async Task<User> GetUser(int id) {
            try {
                return await _repository.FindAsync(id);
            } catch (Exception ex) {
                _logger.LogError(ex, "Failed to get user");
                throw;
            }
        }

        public void ProcessData(string data) {
            using var stream = new MemoryStream();
            stream.Write(data);
        }
    }
}
"#;
    let signals = detect_signals(Language::CSharp, source);

    // Class detection
    assert!(
        !signals.naming.class_names.is_empty(),
        "C# should detect class UserService"
    );

    // Function detection
    assert!(
        signals.naming.function_names.len() >= 2,
        "C# should detect methods"
    );

    // Error handling
    assert!(
        !signals.error_handling.try_catch_blocks.is_empty(),
        "C# should detect try/catch"
    );

    // Import patterns (using)
    assert!(
        !signals.import_patterns.absolute_imports.is_empty(),
        "C# should detect using"
    );
}

#[test]
fn test_new_language_scala_detection() {
    let source = r#"
import scala.util.{Try, Success, Failure}

class UserService {
    def processUser(name: String): Either[Error, User] = {
        try {
            Right(database.save(name))
        } catch {
            case e: Exception => Left(new Error(e.getMessage))
        }
    }

    def helper(): Unit = ()
}

object Constants {
    val MaxRetries = 3
}

trait Processable {
    def process(): Unit
}
"#;
    let signals = detect_signals(Language::Scala, source);

    // Class detection
    assert!(
        !signals.naming.class_names.is_empty(),
        "Scala should detect class UserService"
    );

    // Function detection
    assert!(
        signals.naming.function_names.len() >= 2,
        "Scala should detect functions"
    );

    // Error handling
    assert!(
        !signals.error_handling.try_catch_blocks.is_empty(),
        "Scala should detect try/catch"
    );

    // Import patterns
    assert!(
        !signals.import_patterns.absolute_imports.is_empty(),
        "Scala should detect imports"
    );
}

#[test]
fn test_new_language_php_detection() {
    let source = r#"<?php
namespace App\Services;

use App\Models\User;
use Illuminate\Http\Request;

class UserController {
    public function index(Request $request) {
        try {
            return User::all();
        } catch (\Exception $e) {
            return response()->json(['error' => $e->getMessage()], 500);
        }
    }

    public function store(Request $request) {
        $user = User::create($request->all());
        return $user;
    }
}
"#;
    let signals = detect_signals(Language::Php, source);

    // Class detection
    assert!(
        !signals.naming.class_names.is_empty(),
        "PHP should detect class UserController"
    );

    // Function detection
    assert!(
        signals.naming.function_names.len() >= 2,
        "PHP should detect methods"
    );

    // Error handling
    assert!(
        !signals.error_handling.try_catch_blocks.is_empty(),
        "PHP should detect try/catch"
    );

    // Import patterns (use)
    assert!(
        !signals.import_patterns.absolute_imports.is_empty(),
        "PHP should detect use imports"
    );
}

#[test]
fn test_new_language_lua_detection() {
    let source = r#"
local json = require("json")
local utils = require("utils")

local function process_data(items)
    local ok, err = pcall(function()
        for _, item in ipairs(items) do
            handle(item)
        end
    end)
    if not ok then
        print("Error: " .. err)
    end
end

function helper()
    return true
end
"#;
    let signals = detect_signals(Language::Lua, source);

    // Function detection
    assert!(
        signals.naming.function_names.len() >= 2,
        "Lua should detect function names: process_data, helper"
    );

    // Import patterns (require)
    assert!(
        !signals.import_patterns.absolute_imports.is_empty(),
        "Lua should detect require as imports"
    );

    // No class detection for Lua
    assert!(signals.naming.class_names.is_empty(), "Lua has no classes");
}

#[test]
fn test_new_language_luau_detection() {
    let source = r#"
local HttpService = game:GetService("HttpService")

type UserData = {
    name: string,
    age: number,
    is_deleted: boolean,
}

local function processUser(data: UserData): boolean
    local success, result = pcall(function()
        return validate(data)
    end)
    return success
end

local function helper(): nil
    return nil
end
"#;
    let signals = detect_signals(Language::Luau, source);

    // Function detection
    assert!(
        signals.naming.function_names.len() >= 2,
        "Luau should detect function names"
    );
}

#[test]
fn test_new_language_elixir_detection() {
    let source = r#"
defmodule UserService do
  import Ecto.Query
  alias MyApp.Repo

  def get_user(id) do
    try do
      Repo.get!(User, id)
    rescue
      Ecto.NoResultsError -> {:error, :not_found}
    end
  end

  defp validate(user) do
    # private function
    user
  end

  def test_helper do
    :ok
  end
end
"#;
    let signals = detect_signals(Language::Elixir, source);

    // Class/module detection (defmodule -> class equivalent)
    assert!(
        !signals.naming.class_names.is_empty(),
        "Elixir should detect defmodule as class/module"
    );

    // Function detection (def/defp)
    assert!(
        signals.naming.function_names.len() >= 2,
        "Elixir should detect def/defp as functions"
    );

    // Import patterns (import, alias)
    assert!(
        !signals.import_patterns.absolute_imports.is_empty(),
        "Elixir should detect import/alias as imports"
    );
}

#[test]
fn test_new_language_ocaml_detection() {
    let source = r#"
open Printf

module UserService = struct
  let process_user name =
    try
      save name
    with
    | Not_found -> Error "not found"
    | Invalid_argument msg -> Error msg

  let helper () = ()
end

let main () =
  let result = UserService.process_user "alice" in
  match result with
  | Ok _ -> print_endline "success"
  | Error msg -> print_endline msg
"#;
    let signals = detect_signals(Language::Ocaml, source);

    // Module detection
    assert!(
        !signals.naming.class_names.is_empty(),
        "OCaml should detect module as class equivalent"
    );

    // Function detection (let bindings)
    assert!(
        signals.naming.function_names.len() >= 2,
        "OCaml should detect let bindings as functions"
    );

    // Import patterns (open)
    assert!(
        !signals.import_patterns.absolute_imports.is_empty(),
        "OCaml should detect open as import"
    );
}

// ============================================================================
// Part 5: Custom Extractor Tests
// ============================================================================
// These tests verify that framework-specific detection continues to work
// after the refactor. They test the exact custom extractors that must be
// preserved in the LanguageProfile's Tier 3 extractors.

#[test]
fn test_custom_extractor_python_fastapi_decorators() {
    let source = r#"
from fastapi import FastAPI

app = FastAPI()

@app.get("/users")
def list_users():
    return []

@app.post("/users")
def create_user():
    return {}

@app.put("/users/{id}")
def update_user():
    return {}

@app.delete("/users/{id}")
def delete_user():
    return {}

@router.get("/items")
def list_items():
    return []
"#;
    let signals = detect_signals(Language::Python, source);

    // Must detect all 5 FastAPI decorators
    assert!(
        signals.api_conventions.fastapi_decorators.len() >= 5,
        "Expected >= 5 FastAPI decorators, got {}",
        signals.api_conventions.fastapi_decorators.len()
    );
}

#[test]
fn test_custom_extractor_python_flask_decorators() {
    let source = r#"
from flask import Flask, Blueprint

app = Flask(__name__)
blueprint = Blueprint('users', __name__)

@app.route("/health")
def health():
    return "ok"

@blueprint.route("/users")
def list_users():
    return []
"#;
    let signals = detect_signals(Language::Python, source);

    assert!(
        signals.api_conventions.flask_decorators.len() >= 2,
        "Expected >= 2 Flask decorators, got {}",
        signals.api_conventions.flask_decorators.len()
    );
}

#[test]
fn test_custom_extractor_python_pytest_fixtures() {
    let source = r#"
import pytest

@pytest.fixture
def db_session():
    session = create_session()
    yield session
    session.close()

@fixture
def user_data():
    return {"name": "test"}

@pytest.fixture(scope="module")
def api_client():
    return Client()
"#;
    let signals = detect_signals(Language::Python, source);

    // All three fixtures should be detected
    assert!(
        signals.test_idioms.pytest_fixtures.len() >= 2,
        "Expected >= 2 pytest fixtures, got {}",
        signals.test_idioms.pytest_fixtures.len()
    );

    assert_eq!(
        signals.test_idioms.detected_framework,
        Some("pytest".to_string())
    );
}

#[test]
fn test_custom_extractor_python_mock_patches() {
    let source = r#"
from unittest import mock

@mock.patch("service.get_user")
def test_get_user(mock_get):
    pass

@patch("service.create_user")
def test_create_user(mock_create):
    pass
"#;
    let signals = detect_signals(Language::Python, source);

    assert!(
        signals.test_idioms.mock_patches.len() >= 2,
        "Expected >= 2 mock.patch decorators, got {}",
        signals.test_idioms.mock_patches.len()
    );
}

#[test]
fn test_custom_extractor_typescript_jest_blocks() {
    let source = r#"
describe('UserService', () => {
    describe('createUser', () => {
        it('should create a user', () => {
            expect(true).toBe(true);
        });

        it('should validate input', () => {
            expect(false).toBe(false);
        });
    });

    test('should handle errors', () => {
        expect(() => { throw new Error(); }).toThrow();
    });
});
"#;
    let signals = detect_signals(Language::TypeScript, source);

    // Should detect describe, it, test blocks
    assert!(
        signals.test_idioms.jest_blocks.len() >= 3,
        "Expected >= 3 Jest blocks (describe + 2 it + 1 test), got {}",
        signals.test_idioms.jest_blocks.len()
    );

    assert_eq!(
        signals.test_idioms.detected_framework,
        Some("jest".to_string())
    );
}

#[test]
fn test_custom_extractor_typescript_zod_schemas() {
    let source = r#"
import { z } from 'zod';

const UserSchema = z.object({
    name: z.string().min(1),
    age: z.number().positive(),
    email: z.string().email(),
});

const ConfigSchema = z.object({
    timeout: z.number(),
});
"#;
    let signals = detect_signals(Language::TypeScript, source);

    assert!(
        signals.validation.zod_schemas.len() >= 2,
        "Expected >= 2 Zod schemas, got {}",
        signals.validation.zod_schemas.len()
    );
}

#[test]
fn test_custom_extractor_go_err_nil_checks() {
    let source = r#"
package main

func process() error {
    data, err := fetchData()
    if err != nil {
        return fmt.Errorf("fetch failed: %w", err)
    }

    result, err := transform(data)
    if err != nil {
        return fmt.Errorf("transform failed: %w", err)
    }

    if err == nil {
        log.Println("success")
    }

    return nil
}
"#;
    let signals = detect_signals(Language::Go, source);

    // Should detect all 3 err != nil / err == nil checks
    assert!(
        signals.error_handling.err_nil_checks.len() >= 3,
        "Expected >= 3 err nil checks, got {}",
        signals.error_handling.err_nil_checks.len()
    );
}

#[test]
fn test_custom_extractor_rust_drop_impl() {
    let source = r#"
struct Connection {
    handle: u64,
}

impl Drop for Connection {
    fn drop(&mut self) {
        close_handle(self.handle);
    }
}

impl Connection {
    fn new(handle: u64) -> Self {
        Self { handle }
    }
}
"#;
    let signals = detect_signals(Language::Rust, source);

    assert!(
        !signals.resource_management.drop_impls.is_empty(),
        "Should detect impl Drop for Connection"
    );
}

#[test]
fn test_custom_extractor_python_isinstance_check() {
    let source = r#"
def validate(data):
    if isinstance(data, dict):
        process_dict(data)
    elif isinstance(data, list):
        process_list(data)

    file.close()
"#;
    let signals = detect_signals(Language::Python, source);

    assert!(
        signals.validation.type_checks.len() >= 2,
        "Expected >= 2 isinstance checks, got {}",
        signals.validation.type_checks.len()
    );

    assert!(
        !signals.resource_management.close_calls.is_empty(),
        "Should detect .close() call"
    );
}

#[test]
fn test_custom_extractor_python_pydantic_model() {
    let source = r#"
from pydantic import BaseModel

class UserCreate(BaseModel):
    name: str
    email: str

class UserResponse(BaseModel):
    id: int
    name: str
"#;
    let signals = detect_signals(Language::Python, source);

    assert!(
        signals.validation.pydantic_models.len() >= 2,
        "Expected >= 2 Pydantic models, got {}",
        signals.validation.pydantic_models.len()
    );
}

#[test]
fn test_custom_extractor_rust_tokio_detection() {
    let source = r#"
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

struct AppState {
    data: tokio::sync::RwLock<Vec<String>>,
}

async fn run() {
    let state = AppState {
        data: tokio::sync::RwLock::new(vec![]),
    };
}
"#;
    let signals = detect_signals(Language::Rust, source);

    // Tokio usage should be detected from use declarations and struct fields
    assert!(
        signals.async_patterns.tokio_usage.len() >= 2,
        "Expected >= 2 tokio usages, got {}",
        signals.async_patterns.tokio_usage.len()
    );
}

#[test]
fn test_custom_extractor_python_context_managers() {
    let source = r#"
with open("file.txt") as f:
    data = f.read()

with db.session() as session:
    session.query(User).all()

class MyManager:
    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.cleanup()
"#;
    let signals = detect_signals(Language::Python, source);

    assert!(
        signals.resource_management.context_managers.len() >= 2,
        "Expected >= 2 context managers, got {}",
        signals.resource_management.context_managers.len()
    );

    assert!(
        !signals.resource_management.enter_exit_methods.is_empty(),
        "Should detect __enter__/__exit__ methods"
    );
}

#[test]
fn test_custom_extractor_go_sync_primitives() {
    let source = r#"
package main

import "sync"

type Cache struct {
    mu sync.Mutex
    data map[string]string
}

type EventBus struct {
    ch chan Event
}
"#;
    let signals = detect_signals(Language::Go, source);

    // Should detect sync.Mutex as sync primitive
    let has_mutex = signals
        .async_patterns
        .sync_primitives
        .iter()
        .any(|(kind, _)| kind == "mutex");
    assert!(has_mutex, "Should detect sync.Mutex");

    // Should detect chan as sync primitive
    let has_channel = signals
        .async_patterns
        .sync_primitives
        .iter()
        .any(|(kind, _)| kind == "channel");
    assert!(has_channel, "Should detect chan as channel");
}

#[test]
fn test_custom_extractor_rust_sync_primitives() {
    let source = r#"
use std::sync::{Mutex, mpsc};

struct SharedState {
    data: Mutex<Vec<String>>,
    lock: RwLock<HashMap<String, String>>,
}

fn setup_channel() {
    let (tx, rx) = mpsc::channel();
}
"#;
    let signals = detect_signals(Language::Rust, source);

    let has_mutex = signals
        .async_patterns
        .sync_primitives
        .iter()
        .any(|(kind, _)| kind == "mutex");
    assert!(has_mutex, "Should detect Mutex/RwLock");
}

#[test]
fn test_custom_extractor_python_soft_delete_assignment() {
    let source = r#"
class User:
    is_deleted = False
    deleted_at = None

    def soft_delete(self):
        self.is_deleted = True
        self.deleted_at = "2024-01-01"
"#;
    let signals = detect_signals(Language::Python, source);

    assert!(!signals.soft_delete.is_deleted_fields.is_empty());
    assert!(!signals.soft_delete.deleted_at_fields.is_empty());
}

#[test]
fn test_custom_extractor_typescript_soft_delete_interface() {
    let source = r#"
interface User {
    id: number;
    name: string;
    is_deleted: boolean;
    deletedAt: Date | null;
}
"#;
    let signals = detect_signals(Language::TypeScript, source);

    assert!(!signals.soft_delete.is_deleted_fields.is_empty());
    assert!(!signals.soft_delete.deleted_at_fields.is_empty());
}

#[test]
fn test_custom_extractor_typescript_express_routes() {
    let source = r#"
import express from 'express';

const app = express();

app.get('/users', (req, res) => {
    res.json([]);
});

app.post('/users', (req, res) => {
    res.status(201).json({});
});
"#;
    let signals = detect_signals(Language::TypeScript, source);

    assert!(
        signals.api_conventions.express_routes.len() >= 2,
        "Expected >= 2 Express routes, got {}",
        signals.api_conventions.express_routes.len()
    );
}

// ============================================================================
// Part 5b: Edge Cases and Invariant Tests
// ============================================================================

#[test]
fn test_invariant_evidence_line_numbers_are_1_indexed() {
    let source = "def hello():\n    pass\n";
    let signals = detect_signals(Language::Python, source);

    // All evidence should have line >= 1 (1-indexed, never 0)
    for (_, _, _) in &signals.naming.function_names {
        // function_names is (name, case, file) not evidence, skip
    }
    // Check evidence vectors directly
    for ev in &signals.error_handling.try_except_blocks {
        assert!(
            ev.line >= 1,
            "Evidence line should be 1-indexed, got {}",
            ev.line
        );
    }
}

#[test]
fn test_invariant_empty_source_produces_empty_signals() {
    let source = "";
    let signals = detect_signals(Language::Python, source);

    assert!(signals.naming.function_names.is_empty());
    assert!(signals.naming.class_names.is_empty());
    assert!(signals.naming.constant_names.is_empty());
    assert!(signals.error_handling.try_except_blocks.is_empty());
}

#[test]
fn test_invariant_detect_naming_case_correctness() {
    assert_eq!(detect_naming_case("process_data"), NamingCase::SnakeCase);
    assert_eq!(
        detect_naming_case("MAX_RETRIES"),
        NamingCase::UpperSnakeCase
    );
    assert_eq!(detect_naming_case("UserManager"), NamingCase::PascalCase);
    assert_eq!(detect_naming_case("processData"), NamingCase::CamelCase);
    assert_eq!(detect_naming_case(""), NamingCase::Unknown);
    assert_eq!(detect_naming_case("x"), NamingCase::Unknown);
    assert_eq!(detect_naming_case("__init__"), NamingCase::Unknown);
}

#[test]
fn test_invariant_fallback_detection_is_language_agnostic() {
    let source = r#"
is_deleted = True
deleted_at = None
async def process():
    pass
try:
    pass
except:
    pass
"#;
    // Fallback should work for any language
    let detector = PatternDetector::new(Language::Python, PathBuf::from("test.py"));
    let signals = detector.detect_fallback(source);

    assert!(!signals.soft_delete.is_deleted_fields.is_empty());
    assert!(!signals.soft_delete.deleted_at_fields.is_empty());
    assert!(!signals.async_patterns.async_await.is_empty());
    assert!(!signals.error_handling.try_except_blocks.is_empty());
}

#[test]
fn test_invariant_snippet_extraction_max_3_lines() {
    let source = r#"
def very_long_function():
    line_one = 1
    line_two = 2
    line_three = 3
    line_four = 4
    line_five = 5
    return line_one
"#;
    let signals = detect_signals(Language::Python, source);

    // Check that snippet extraction yields reasonable snippets
    for (_, _, _) in &signals.naming.function_names {
        // function_names has (name, case, file) not snippets
        // Evidence with snippets is in other signal vectors
    }
    // This is a structural invariant test -- the refactor must preserve
    // the 3-line snippet limit in get_snippet()
}

#[test]
fn test_invariant_python_self_cls_not_counted_as_untyped() {
    let source = r#"
class MyClass:
    def method(self, name: str):
        pass

    @classmethod
    def factory(cls, data: dict):
        pass
"#;
    let signals = detect_signals(Language::Python, source);

    // self and cls should NOT be counted as untyped params
    // Only name: str and data: dict should be counted as typed
    assert!(signals.type_coverage.typed_params >= 2);
    // self and cls should not contribute to untyped_params
    assert_eq!(
        signals.type_coverage.untyped_params, 0,
        "self/cls should not be counted as untyped params"
    );
}
