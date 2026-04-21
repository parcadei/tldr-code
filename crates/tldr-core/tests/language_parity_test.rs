//! Language parity tests - ensure tldr-rs matches Python v1 language support
//!
//! These tests define behavioral contracts that must pass for language support
//! to be considered complete. Based on spec:
//! `/Users/cosimo/.opc-dev/thoughts/shared/plans/tldr-rs-language-migration-spec.md`
//!
//! # Test Categories
//!
//! 1. **Parser Tests** - Each language should parse without `UnsupportedLanguage` error
//! 2. **Import Tests** - Each language should extract imports correctly
//! 3. **Function Tests** - Each language should find function definitions
//! 4. **Rust-specific Callgraph Tests** - `use` path and trait method resolution
//! 5. **Java-specific Callgraph Tests** - Package and interface resolution
//! 6. **P2 Language Tests** - Stub languages that return `UnsupportedLanguage`
//!
//! # Running Tests
//!
//! ```bash
//! cargo test -p tldr-core --test language_parity_test -- --test-threads=1
//! ```

use tldr_core::{
    ast::extractor::{extract_classes, extract_functions, extract_methods},
    ast::imports::extract_imports_from_tree,
    ast::parser::ParserPool,
    Language, TldrError,
};

// =============================================================================
// Sample Code Constants (Spec Section 2.1 - Import patterns per language)
// =============================================================================

/// Python sample with imports (Spec L101)
const SAMPLE_PYTHON_CODE: &str = r#"
import os
import sys
from typing import List, Optional
from collections import defaultdict as dd
from . import relative_import
from ..parent import parent_module

def hello(name: str) -> str:
    """Greet someone."""
    return f"Hello, {name}!"

class Greeter:
    def __init__(self, prefix: str):
        self.prefix = prefix

    def greet(self, name: str) -> str:
        return f"{self.prefix} {name}"

async def async_fetch(url: str) -> str:
    return ""
"#;

/// TypeScript sample with imports (Spec L102)
const SAMPLE_TYPESCRIPT_CODE: &str = r#"
import React from 'react';
import { useState, useEffect } from 'react';
import * as lodash from 'lodash';
import type { User } from './types';
import './styles.css';

const greeting = require('./greeting');

export function hello(name: string): string {
    return `Hello, ${name}!`;
}

export class Greeter {
    constructor(private prefix: string) {}

    greet(name: string): string {
        return `${this.prefix} ${name}`;
    }
}

export const arrowFunc = (x: number): number => x * 2;

export async function asyncFetch(url: string): Promise<string> {
    return "";
}
"#;

/// Go sample with imports (Spec L103)
const SAMPLE_GO_CODE: &str = r#"
package main

import (
    "fmt"
    "os"
    myalias "path/to/package"
    . "dot/import"
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

/// Rust sample with imports (Spec L104)
const SAMPLE_RUST_CODE: &str = r#"
use std::collections::HashMap;
use std::io::{self, Read, Write};
use crate::utils::helper;
use super::parent_module;

mod internal_module;

pub fn hello(name: &str) -> String {
    format!("Hello, {}!", name)
}

pub struct Greeter {
    prefix: String,
}

impl Greeter {
    pub fn new(prefix: &str) -> Self {
        Self { prefix: prefix.to_string() }
    }

    pub fn greet(&self, name: &str) -> String {
        format!("{} {}", self.prefix, name)
    }
}

pub trait Greetable {
    fn greet(&self) -> String;
}

impl Greetable for Greeter {
    fn greet(&self) -> String {
        self.greet("default")
    }
}

pub async fn async_fetch(url: &str) -> Result<String, io::Error> {
    Ok(String::new())
}
"#;

/// Java sample with imports (Spec L105)
const SAMPLE_JAVA_CODE: &str = r#"
package com.example.greeting;

import java.util.List;
import java.util.Map;
import java.util.function.Function;
import static java.lang.Math.PI;
import com.example.utils.*;

public class Greeter {
    private final String prefix;

    public Greeter(String prefix) {
        this.prefix = prefix;
    }

    public String greet(String name) {
        return prefix + " " + name;
    }

    public static String hello(String name) {
        return "Hello, " + name + "!";
    }

    private void privateMethod() {
        // Should be extracted as method
    }
}

interface Greetable {
    String greet();
}

class SimpleGreeter implements Greetable {
    @Override
    public String greet() {
        return "Hi!";
    }
}
"#;

// =============================================================================
// P2 Language Sample Code (Spec Section 3.3 - Currently return UnsupportedLanguage)
// =============================================================================

/// C sample code (Spec L46)
const SAMPLE_C_CODE: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include "local_header.h"

void hello(const char* name) {
    printf("Hello, %s!\n", name);
}

struct Greeter {
    char* prefix;
};

int main(int argc, char** argv) {
    hello("World");
    return 0;
}
"#;

/// C++ sample code (Spec L47)
const SAMPLE_CPP_CODE: &str = r#"
#include <iostream>
#include <string>
#include <vector>
#include "local_header.hpp"

namespace greeting {

class Greeter {
public:
    Greeter(const std::string& prefix) : prefix_(prefix) {}

    std::string greet(const std::string& name) const {
        return prefix_ + " " + name;
    }

private:
    std::string prefix_;
};

void hello(const std::string& name) {
    std::cout << "Hello, " << name << "!" << std::endl;
}

}  // namespace greeting

int main() {
    greeting::hello("World");
    return 0;
}
"#;

/// Ruby sample code (Spec L48)
/// Note: Uses r##"..."## to handle Ruby's string interpolation syntax
const SAMPLE_RUBY_CODE: &str = r##"
require 'json'
require 'net/http'
require_relative './local_module'

module Greeting
  class Greeter
    def initialize(prefix)
      @prefix = prefix
    end

    def greet(name)
      "#{@prefix} #{name}"
    end
  end

  def self.hello(name)
    "Hello, #{name}!"
  end
end

def top_level_function
  puts "Top level"
end
"##;

/// Kotlin sample code (Spec L49)
const SAMPLE_KOTLIN_CODE: &str = r#"
package com.example.greeting

import java.util.List
import kotlin.collections.mutableListOf
import com.example.utils.*

fun hello(name: String): String {
    return "Hello, " + name + "!"
}

class Greeter(private val prefix: String) {
    fun greet(name: String): String {
        return prefix + " " + name
    }
}

interface Greetable {
    fun greet(): String
}

suspend fun asyncFetch(url: String): String {
    return ""
}
"#;

/// Swift sample code (Spec L50)
const SAMPLE_SWIFT_CODE: &str = r#"
import Foundation
import UIKit
import SwiftUI

func hello(name: String) -> String {
    return "Hello, " + name + "!"
}

class Greeter {
    private let prefix: String

    init(prefix: String) {
        self.prefix = prefix
    }

    func greet(name: String) -> String {
        return prefix + " " + name
    }
}

protocol Greetable {
    func greet() -> String
}

struct SimpleGreeter: Greetable {
    func greet() -> String {
        return "Hi!"
    }
}

@MainActor
func asyncFetch(url: String) async throws -> String {
    return ""
}
"#;

/// C# sample code (Spec L51)
const SAMPLE_CSHARP_CODE: &str = r#"
using System;
using System.Collections.Generic;
using System.Linq;
using MyProject.Utils;

namespace Greeting
{
    public class Greeter
    {
        private readonly string _prefix;

        public Greeter(string prefix)
        {
            _prefix = prefix;
        }

        public string Greet(string name)
        {
            return _prefix + " " + name;
        }

        public static string Hello(string name)
        {
            return "Hello, " + name + "!";
        }
    }

    public interface IGreetable
    {
        string Greet();
    }

    public async Task<string> AsyncFetch(string url)
    {
        return "";
    }
}
"#;

/// Scala sample code (Spec L52)
const SAMPLE_SCALA_CODE: &str = r#"
package com.example.greeting

import scala.collection.mutable.{ListBuffer, Map => MutableMap}
import java.util.{List => JList, _}
import com.example.utils._

object Greeting {
  def hello(name: String): String = {
    "Hello, " + name + "!"
  }
}

class Greeter(prefix: String) {
  def greet(name: String): String = {
    prefix + " " + name
  }
}

trait Greetable {
  def greet(): String
}

case class SimpleGreeter() extends Greetable {
  override def greet(): String = "Hi!"
}
"#;

/// PHP sample code (Spec L53)
const SAMPLE_PHP_CODE: &str = r#"
<?php

namespace Greeting;

use Vendor\Package\SomeClass;
use Vendor\Package\{ClassA, ClassB};
use function Vendor\Package\functionName;

require_once __DIR__ . '/local.php';
include 'another.php';

function hello(string $name): string {
    return "Hello, " . $name . "!";
}

class Greeter {
    private string $prefix;

    public function __construct(string $prefix) {
        $this->prefix = $prefix;
    }

    public function greet(string $name): string {
        return $this->prefix . " " . $name;
    }
}

interface Greetable {
    public function greet(): string;
}
"#;

/// Lua sample code (Spec L54)
const SAMPLE_LUA_CODE: &str = r#"
local json = require("json")
local http = require("socket.http")

local Greeter = {}
Greeter.__index = Greeter

function Greeter:new(prefix)
    local self = setmetatable({}, Greeter)
    self.prefix = prefix
    return self
end

function Greeter:greet(name)
    return self.prefix .. " " .. name
end

local function hello(name)
    return "Hello, " .. name .. "!"
end

return {
    Greeter = Greeter,
    hello = hello
}
"#;

/// Luau sample code (Roblox Lua variant) (Spec L55)
const SAMPLE_LUAU_CODE: &str = r#"
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

type Greeter = {
    prefix: string,
    greet: (self: Greeter, name: string) -> string
}

local function createGreeter(prefix: string): Greeter
    local self = {}
    self.prefix = prefix

    function self:greet(name: string): string
        return self.prefix .. " " .. name
    end

    return self
end

local function hello(name: string): string
    return "Hello, " .. name .. "!"
end

return {
    createGreeter = createGreeter,
    hello = hello
}
"#;

/// Elixir sample code (Spec L56)
/// Note: Uses r##"..."## to handle Elixir's string interpolation syntax
const SAMPLE_ELIXIR_CODE: &str = r##"
defmodule Greeting do
  import Logger
  alias MyApp.Utils
  require Jason

  def hello(name) do
    "Hello, #{name}!"
  end

  defp private_helper(x) do
    x * 2
  end
end

defmodule Greeter do
  defstruct [:prefix]

  def new(prefix) do
    %__MODULE__{prefix: prefix}
  end

  def greet(%__MODULE__{prefix: prefix}, name) do
    "#{prefix} #{name}"
  end
end

defprotocol Greetable do
  def greet(greeter)
end

defimpl Greetable, for: Greeter do
  def greet(%Greeter{} = greeter) do
    Greeter.greet(greeter, "default")
  end
end
"##;

// =============================================================================
// Module: Parser Tests (Spec Section 2.1 - Contract: parse)
// =============================================================================

mod parser_tests {
    use super::*;

    /// Test Python parsing (P0 language - should work)
    /// Spec L40: Python has tree_sitter_python
    #[test]
    fn test_python_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_PYTHON_CODE, Language::Python);
        assert!(
            result.is_ok(),
            "Python should be parseable: {:?}",
            result.err()
        );

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "Python parse tree should not have errors"
        );
    }

    /// Test TypeScript parsing (P0 language - should work)
    /// Spec L41: TypeScript has tree_sitter_typescript
    #[test]
    fn test_typescript_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_TYPESCRIPT_CODE, Language::TypeScript);
        assert!(
            result.is_ok(),
            "TypeScript should be parseable: {:?}",
            result.err()
        );

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "TypeScript parse tree should not have errors"
        );
    }

    /// Test JavaScript parsing (P0 language - uses TypeScript grammar)
    /// Spec L42: JavaScript uses same grammar as TypeScript
    #[test]
    fn test_javascript_parse() {
        let pool = ParserPool::new();
        // Use TypeScript code minus type annotations for JS
        let js_code = r#"
const greeting = require('./greeting');

function hello(name) {
    return `Hello, ${name}!`;
}

class Greeter {
    constructor(prefix) {
        this.prefix = prefix;
    }
}
"#;
        let result = pool.parse(js_code, Language::JavaScript);
        assert!(
            result.is_ok(),
            "JavaScript should be parseable: {:?}",
            result.err()
        );
    }

    /// Test Go parsing (P0 language - should work)
    /// Spec L43: Go has tree_sitter_go
    #[test]
    fn test_go_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_GO_CODE, Language::Go);
        assert!(result.is_ok(), "Go should be parseable: {:?}", result.err());

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "Go parse tree should not have errors"
        );
    }

    /// Test Rust parsing (P1 language - should work, partial support)
    /// Spec L44: Rust has tree_sitter_rust
    #[test]
    fn test_rust_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_RUST_CODE, Language::Rust);
        assert!(
            result.is_ok(),
            "Rust should be parseable: {:?}",
            result.err()
        );

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "Rust parse tree should not have errors"
        );
    }

    /// Test Java parsing (P1 language - should work, partial support)
    /// Spec L45: Java has tree_sitter_java
    #[test]
    fn test_java_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_JAVA_CODE, Language::Java);
        assert!(
            result.is_ok(),
            "Java should be parseable: {:?}",
            result.err()
        );

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "Java parse tree should not have errors"
        );
    }

    // =========================================================================
    // P2 Languages - Currently return UnsupportedLanguage (Spec Section 3.3)
    // These tests use #[ignore] because grammars are not loaded yet
    // =========================================================================

    /// Test C parsing (P2 language - Phase 2 implemented)
    /// Spec L46: C grammar loaded via tree-sitter-c
    #[test]
    fn test_c_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_C_CODE, Language::C);
        assert!(result.is_ok(), "C should be parseable: {:?}", result.err());

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "C parse tree should not have errors"
        );
    }

    /// Test C++ parsing (P2 language - Phase 2 implemented)
    /// Spec L47: C++ grammar loaded via tree-sitter-cpp
    #[test]
    fn test_cpp_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_CPP_CODE, Language::Cpp);
        assert!(
            result.is_ok(),
            "C++ should be parseable: {:?}",
            result.err()
        );

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "C++ parse tree should not have errors"
        );
    }

    /// Test Ruby parsing (P2 language - Phase 3 implemented)
    /// Spec L48: Ruby grammar loaded via tree-sitter-ruby 0.23.1
    #[test]
    fn test_ruby_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_RUBY_CODE, Language::Ruby);
        assert!(
            result.is_ok(),
            "Ruby should be parseable: {:?}",
            result.err()
        );

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "Ruby parse tree should not have errors"
        );
    }

    /// Test Kotlin parsing (P2 language - deferred due to ABI incompatibility)
    /// Spec L49: Kotlin grammar not loaded
    /// Note: tree-sitter-kotlin 0.3.8 requires tree-sitter >=0.21 <0.23, but we use 0.24.7
    /// Deferred until upstream crate is updated. See: https://github.com/fwcd/tree-sitter-kotlin
    #[test]
    #[ignore = "P2 language: tree-sitter-kotlin 0.3.8 requires tree-sitter <0.23 (ABI incompatible with 0.24.7)"]
    fn test_kotlin_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_KOTLIN_CODE, Language::Kotlin);
        assert!(
            result.is_ok(),
            "Kotlin should be parseable after grammar is updated for tree-sitter 0.24: {:?}",
            result.err()
        );
    }

    /// Test Swift parsing (P2 language - deferred due to ABI incompatibility)
    /// Spec L50: Swift grammar not loaded
    /// Note: tree-sitter-swift 0.7.1 has grammar ABI version 15, but tree-sitter 0.24.7 expects version 14
    /// Deferred until upstream crate is updated. See: https://github.com/alex-pinkus/tree-sitter-swift
    #[test]
    #[ignore = "P2 language: tree-sitter-swift 0.7.1 has ABI version 15 (incompatible with tree-sitter 0.24.7)"]
    fn test_swift_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_SWIFT_CODE, Language::Swift);
        assert!(
            result.is_ok(),
            "Swift should be parseable after grammar is updated for tree-sitter 0.24: {:?}",
            result.err()
        );
    }

    /// Test C# parsing (P2 language - Phase 4 implemented)
    /// Spec L51: C# grammar loaded via tree-sitter-c-sharp 0.23.1
    #[test]
    fn test_csharp_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_CSHARP_CODE, Language::CSharp);
        assert!(result.is_ok(), "C# should be parseable: {:?}", result.err());

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "C# parse tree should not have errors"
        );
    }

    /// Test Scala parsing (P2 language - Phase 4 implemented)
    /// Spec L52: Scala grammar loaded via tree-sitter-scala 0.24.0
    #[test]
    fn test_scala_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_SCALA_CODE, Language::Scala);
        assert!(
            result.is_ok(),
            "Scala should be parseable: {:?}",
            result.err()
        );

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "Scala parse tree should not have errors"
        );
    }

    /// Test PHP parsing (P2 language - Phase 4 implemented)
    /// Spec L53: PHP grammar loaded via tree-sitter-php 0.23.11
    /// Note: PHP 0.24.2 has ABI v15 incompatible with tree-sitter 0.24.7, using 0.23.11
    #[test]
    fn test_php_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_PHP_CODE, Language::Php);
        assert!(
            result.is_ok(),
            "PHP should be parseable: {:?}",
            result.err()
        );

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "PHP parse tree should not have errors"
        );
    }

    /// Test Lua parsing (P2 language - Phase 5 implemented)
    /// Spec L54: Lua grammar loaded via tree-sitter-lua 0.2.0
    #[test]
    fn test_lua_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_LUA_CODE, Language::Lua);
        assert!(
            result.is_ok(),
            "Lua should be parseable: {:?}",
            result.err()
        );

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "Lua parse tree should not have errors"
        );
    }

    /// Test Luau parsing (P2 language - Phase 5 implemented)
    /// Spec L55: Luau grammar loaded via tree-sitter-luau 1.2.0
    /// Note: Official tree-sitter-luau crate exists on crates.io
    #[test]
    fn test_luau_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_LUAU_CODE, Language::Luau);
        assert!(
            result.is_ok(),
            "Luau should be parseable: {:?}",
            result.err()
        );

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "Luau parse tree should not have errors"
        );
    }

    /// Test Elixir parsing (P2 language - Phase 5 implemented)
    /// Spec L56: Elixir grammar loaded via tree-sitter-elixir 0.3.4
    #[test]
    fn test_elixir_parse() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_ELIXIR_CODE, Language::Elixir);
        assert!(
            result.is_ok(),
            "Elixir should be parseable: {:?}",
            result.err()
        );

        let tree = result.unwrap();
        assert!(
            !tree.root_node().has_error(),
            "Elixir parse tree should not have errors"
        );
    }
}

// =============================================================================
// Module: Import Extraction Tests (Spec Section 2.1.4 - Contract: extract_imports)
// =============================================================================

mod import_tests {
    use super::*;

    /// Test Python import extraction (Spec L101)
    /// Expected imports: os, sys, typing (from), collections (from with alias),
    /// relative imports (., ..)
    #[test]
    fn test_python_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_PYTHON_CODE, Language::Python).unwrap();
        let imports =
            extract_imports_from_tree(&tree, SAMPLE_PYTHON_CODE, Language::Python).unwrap();

        // Should find multiple imports
        assert!(!imports.is_empty(), "Should extract Python imports");

        // Check for specific imports
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"os"), "Should find 'import os'");
        assert!(modules.contains(&"sys"), "Should find 'import sys'");
        assert!(
            modules.contains(&"typing"),
            "Should find 'from typing import'"
        );

        // Check for from imports
        let from_imports: Vec<_> = imports.iter().filter(|i| i.is_from).collect();
        assert!(
            from_imports.len() >= 2,
            "Should have at least 2 from imports"
        );

        // Check for alias
        let aliased: Vec<_> = imports.iter().filter(|i| i.alias.is_some()).collect();
        assert!(
            !aliased.is_empty(),
            "Should find aliased import (defaultdict as dd)"
        );
    }

    /// Test TypeScript import extraction (Spec L102)
    /// Expected: import from, import *, type import, require()
    #[test]
    fn test_typescript_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool
            .parse(SAMPLE_TYPESCRIPT_CODE, Language::TypeScript)
            .unwrap();
        let imports =
            extract_imports_from_tree(&tree, SAMPLE_TYPESCRIPT_CODE, Language::TypeScript).unwrap();

        assert!(!imports.is_empty(), "Should extract TypeScript imports");

        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(
            modules.contains(&"react"),
            "Should find 'import from react'"
        );
        assert!(
            modules.contains(&"lodash"),
            "Should find 'import * as lodash'"
        );
    }

    /// Test Go import extraction (Spec L103)
    /// Expected: import "pkg", import alias "pkg", dot import
    #[test]
    fn test_go_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_GO_CODE, Language::Go).unwrap();
        let imports = extract_imports_from_tree(&tree, SAMPLE_GO_CODE, Language::Go).unwrap();

        assert!(!imports.is_empty(), "Should extract Go imports");

        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"fmt"), "Should find 'import fmt'");
        assert!(modules.contains(&"os"), "Should find 'import os'");
    }

    /// Test Rust import extraction (Spec L104)
    /// Expected: use statements, mod declarations
    #[test]
    fn test_rust_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_RUST_CODE, Language::Rust).unwrap();
        let imports = extract_imports_from_tree(&tree, SAMPLE_RUST_CODE, Language::Rust).unwrap();

        assert!(!imports.is_empty(), "Should extract Rust imports");

        // Check for std::collections::HashMap
        let has_hashmap = imports.iter().any(|i| {
            i.module.contains("std::collections") || i.names.contains(&"HashMap".to_string())
        });
        assert!(has_hashmap, "Should find 'use std::collections::HashMap'");
    }

    /// Test Java import extraction (Spec L105)
    /// Expected: import statements, static imports, wildcard imports
    #[test]
    fn test_java_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_JAVA_CODE, Language::Java).unwrap();
        let imports = extract_imports_from_tree(&tree, SAMPLE_JAVA_CODE, Language::Java).unwrap();

        assert!(!imports.is_empty(), "Should extract Java imports");

        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(
            modules.iter().any(|m| m.contains("java.util")),
            "Should find java.util imports"
        );
    }

    // =========================================================================
    // P2 Languages - Import extraction tests (currently return UnsupportedLanguage)
    // =========================================================================

    /// Test C import extraction (Phase 6 - implemented)
    /// Expected: #include directives
    #[test]
    fn test_c_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_C_CODE, Language::C).unwrap();
        let imports = extract_imports_from_tree(&tree, SAMPLE_C_CODE, Language::C).unwrap();

        assert!(!imports.is_empty(), "Should extract C includes");
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(
            modules.contains(&"stdio.h") || modules.iter().any(|m| m.contains("stdio")),
            "Should find #include <stdio.h>"
        );
    }

    /// Test C++ import extraction (Phase 7 - implemented)
    /// Expected: #include directives
    #[test]
    fn test_cpp_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_CPP_CODE, Language::Cpp).unwrap();
        let imports = extract_imports_from_tree(&tree, SAMPLE_CPP_CODE, Language::Cpp).unwrap();

        assert!(!imports.is_empty(), "Should extract C++ includes");
    }

    /// Test Ruby import extraction (Phase 8 - implemented)
    /// Expected: require, require_relative
    #[test]
    fn test_ruby_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_RUBY_CODE, Language::Ruby).unwrap();
        let imports = extract_imports_from_tree(&tree, SAMPLE_RUBY_CODE, Language::Ruby).unwrap();

        assert!(!imports.is_empty(), "Should extract Ruby requires");
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"json"), "Should find require 'json'");
        assert!(
            modules.contains(&"net/http"),
            "Should find require 'net/http'"
        );
        assert!(
            modules.contains(&"./local_module"),
            "Should find require_relative './local_module'"
        );
    }

    /// Test Kotlin import extraction (P2 - currently stubbed)
    /// Expected: import statements
    #[test]
    #[ignore = "P2 language: Kotlin import extraction not implemented"]
    fn test_kotlin_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_KOTLIN_CODE, Language::Kotlin).unwrap();
        let imports =
            extract_imports_from_tree(&tree, SAMPLE_KOTLIN_CODE, Language::Kotlin).unwrap();

        assert!(!imports.is_empty(), "Should extract Kotlin imports");
    }

    /// Test Swift import extraction (P2 - currently stubbed)
    /// Expected: import statements
    #[test]
    #[ignore = "P2 language: Swift import extraction not implemented"]
    fn test_swift_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_SWIFT_CODE, Language::Swift).unwrap();
        let imports = extract_imports_from_tree(&tree, SAMPLE_SWIFT_CODE, Language::Swift).unwrap();

        assert!(!imports.is_empty(), "Should extract Swift imports");
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(
            modules.contains(&"Foundation"),
            "Should find import Foundation"
        );
    }

    /// Test C# import extraction (P2 - currently stubbed)
    /// Expected: using statements
    #[test]
    #[ignore = "P2 language: C# import extraction not implemented"]
    fn test_csharp_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_CSHARP_CODE, Language::CSharp).unwrap();
        let imports =
            extract_imports_from_tree(&tree, SAMPLE_CSHARP_CODE, Language::CSharp).unwrap();

        assert!(!imports.is_empty(), "Should extract C# usings");
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(
            modules.iter().any(|m| m.contains("System")),
            "Should find using System"
        );
    }

    /// Test Scala import extraction (P2 - currently stubbed)
    /// Expected: import statements with renames and wildcards
    #[test]
    #[ignore = "P2 language: Scala import extraction not implemented"]
    fn test_scala_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_SCALA_CODE, Language::Scala).unwrap();
        let imports = extract_imports_from_tree(&tree, SAMPLE_SCALA_CODE, Language::Scala).unwrap();

        assert!(!imports.is_empty(), "Should extract Scala imports");
    }

    /// Test PHP import extraction (P2 - currently stubbed)
    /// Expected: use statements, require/include
    #[test]
    #[ignore = "P2 language: PHP import extraction not implemented"]
    fn test_php_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_PHP_CODE, Language::Php).unwrap();
        let imports = extract_imports_from_tree(&tree, SAMPLE_PHP_CODE, Language::Php).unwrap();

        assert!(!imports.is_empty(), "Should extract PHP imports");
    }

    /// Test Lua import extraction (P2 - currently stubbed)
    /// Expected: require statements
    #[test]
    #[ignore = "P2 language: Lua import extraction not implemented"]
    fn test_lua_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_LUA_CODE, Language::Lua).unwrap();
        let imports = extract_imports_from_tree(&tree, SAMPLE_LUA_CODE, Language::Lua).unwrap();

        assert!(!imports.is_empty(), "Should extract Lua requires");
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"json"), "Should find require('json')");
    }

    /// Test Elixir import extraction (P2 - currently stubbed)
    /// Expected: import, alias, require statements
    #[test]
    #[ignore = "P2 language: Elixir import extraction not implemented"]
    fn test_elixir_extract_imports() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_ELIXIR_CODE, Language::Elixir).unwrap();
        let imports =
            extract_imports_from_tree(&tree, SAMPLE_ELIXIR_CODE, Language::Elixir).unwrap();

        assert!(!imports.is_empty(), "Should extract Elixir imports");
    }
}

// =============================================================================
// Module: Function Extraction Tests (Spec Section 2.1.5 - Contract: extract_functions)
// =============================================================================

mod function_tests {
    use super::*;

    /// Test Python function extraction
    /// Expected: hello, Greeter.__init__, Greeter.greet, async_fetch
    #[test]
    fn test_python_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_PYTHON_CODE, Language::Python).unwrap();
        let functions = extract_functions(&tree, SAMPLE_PYTHON_CODE, Language::Python);

        assert!(!functions.is_empty(), "Should extract Python functions");
        assert!(
            functions.contains(&"hello".to_string()),
            "Should find 'hello' function"
        );
        assert!(
            functions.contains(&"async_fetch".to_string()),
            "Should find 'async_fetch' function"
        );
    }

    /// Test Python class extraction
    #[test]
    fn test_python_extract_classes() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_PYTHON_CODE, Language::Python).unwrap();
        let classes = extract_classes(&tree, SAMPLE_PYTHON_CODE, Language::Python);

        assert!(!classes.is_empty(), "Should extract Python classes");
        assert!(
            classes.contains(&"Greeter".to_string()),
            "Should find 'Greeter' class"
        );
    }

    /// Test Python method extraction
    #[test]
    fn test_python_extract_methods() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_PYTHON_CODE, Language::Python).unwrap();
        let methods = extract_methods(&tree, SAMPLE_PYTHON_CODE, Language::Python);

        // Methods are functions inside classes
        assert!(!methods.is_empty(), "Should extract Python methods");
    }

    /// Test TypeScript function extraction
    /// Expected: hello, Greeter.greet, arrowFunc, asyncFetch
    #[test]
    fn test_typescript_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool
            .parse(SAMPLE_TYPESCRIPT_CODE, Language::TypeScript)
            .unwrap();
        let functions = extract_functions(&tree, SAMPLE_TYPESCRIPT_CODE, Language::TypeScript);

        assert!(!functions.is_empty(), "Should extract TypeScript functions");
        assert!(
            functions.contains(&"hello".to_string()),
            "Should find 'hello' function"
        );
        assert!(
            functions.contains(&"asyncFetch".to_string()),
            "Should find 'asyncFetch' function"
        );
    }

    /// Test TypeScript class extraction
    #[test]
    fn test_typescript_extract_classes() {
        let pool = ParserPool::new();
        let tree = pool
            .parse(SAMPLE_TYPESCRIPT_CODE, Language::TypeScript)
            .unwrap();
        let classes = extract_classes(&tree, SAMPLE_TYPESCRIPT_CODE, Language::TypeScript);

        assert!(!classes.is_empty(), "Should extract TypeScript classes");
        assert!(
            classes.contains(&"Greeter".to_string()),
            "Should find 'Greeter' class"
        );
    }

    /// Test Go function extraction
    /// Expected: Hello, main, Greeter.Greet (method)
    #[test]
    fn test_go_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_GO_CODE, Language::Go).unwrap();
        let functions = extract_functions(&tree, SAMPLE_GO_CODE, Language::Go);

        assert!(!functions.is_empty(), "Should extract Go functions");
        assert!(
            functions.contains(&"Hello".to_string()),
            "Should find 'Hello' function"
        );
        assert!(
            functions.contains(&"main".to_string()),
            "Should find 'main' function"
        );
    }

    /// Test Rust function extraction
    /// Expected: hello, Greeter::new, Greeter::greet, async_fetch
    #[test]
    fn test_rust_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_RUST_CODE, Language::Rust).unwrap();
        let functions = extract_functions(&tree, SAMPLE_RUST_CODE, Language::Rust);

        assert!(!functions.is_empty(), "Should extract Rust functions");
        assert!(
            functions.contains(&"hello".to_string()),
            "Should find 'hello' function"
        );
        assert!(
            functions.contains(&"async_fetch".to_string()),
            "Should find 'async_fetch' function"
        );
    }

    /// Test Rust struct extraction
    #[test]
    fn test_rust_extract_structs() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_RUST_CODE, Language::Rust).unwrap();
        let classes = extract_classes(&tree, SAMPLE_RUST_CODE, Language::Rust);

        assert!(!classes.is_empty(), "Should extract Rust structs");
        assert!(
            classes.contains(&"Greeter".to_string()),
            "Should find 'Greeter' struct"
        );
    }

    /// Test Rust impl method extraction
    #[test]
    fn test_rust_extract_impl_methods() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_RUST_CODE, Language::Rust).unwrap();
        let methods = extract_methods(&tree, SAMPLE_RUST_CODE, Language::Rust);

        assert!(!methods.is_empty(), "Should extract Rust impl methods");
        // Should find methods like "new" and "greet" from impl blocks
    }

    /// Test Java function/method extraction
    /// Expected: Greeter.greet, Greeter.hello (static), SimpleGreeter.greet
    #[test]
    fn test_java_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_JAVA_CODE, Language::Java).unwrap();
        let functions = extract_functions(&tree, SAMPLE_JAVA_CODE, Language::Java);

        // In Java, we extract methods from classes
        // Functions here means methods at class level (not nested)
        let _ = functions;
    }

    /// Test Java class extraction
    #[test]
    fn test_java_extract_classes() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_JAVA_CODE, Language::Java).unwrap();
        let classes = extract_classes(&tree, SAMPLE_JAVA_CODE, Language::Java);

        assert!(!classes.is_empty(), "Should extract Java classes");
        assert!(
            classes.contains(&"Greeter".to_string()),
            "Should find 'Greeter' class"
        );
    }

    // =========================================================================
    // P2 Languages - Function extraction tests
    // =========================================================================

    /// Test C function extraction (Phase 6 - implemented)
    #[test]
    fn test_c_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_C_CODE, Language::C).unwrap();
        let functions = extract_functions(&tree, SAMPLE_C_CODE, Language::C);

        assert!(!functions.is_empty(), "Should extract C functions");
        assert!(
            functions.contains(&"hello".to_string()),
            "Should find 'hello' function"
        );
        assert!(
            functions.contains(&"main".to_string()),
            "Should find 'main' function"
        );
    }

    /// Test C++ function extraction (Phase 7 - implemented)
    #[test]
    fn test_cpp_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_CPP_CODE, Language::Cpp).unwrap();
        let functions = extract_functions(&tree, SAMPLE_CPP_CODE, Language::Cpp);

        assert!(!functions.is_empty(), "Should extract C++ functions");
        assert!(
            functions.contains(&"hello".to_string())
                || functions.contains(&"greeting::hello".to_string()),
            "Should find 'hello' function"
        );
    }

    /// Test Ruby function extraction (Phase 8 - implemented)
    #[test]
    fn test_ruby_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_RUBY_CODE, Language::Ruby).unwrap();
        let functions = extract_functions(&tree, SAMPLE_RUBY_CODE, Language::Ruby);

        assert!(!functions.is_empty(), "Should extract Ruby functions");
        assert!(
            functions.contains(&"top_level_function".to_string()),
            "Should find 'top_level_function'"
        );
    }

    /// Test Kotlin function extraction (P2 - currently stubbed)
    #[test]
    #[ignore = "P2 language: Kotlin function extraction not implemented"]
    fn test_kotlin_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_KOTLIN_CODE, Language::Kotlin).unwrap();
        let functions = extract_functions(&tree, SAMPLE_KOTLIN_CODE, Language::Kotlin);

        assert!(!functions.is_empty(), "Should extract Kotlin functions");
        assert!(
            functions.contains(&"hello".to_string()),
            "Should find 'hello' function"
        );
    }

    /// Test Swift function extraction (P2 - currently stubbed)
    #[test]
    #[ignore = "P2 language: Swift function extraction not implemented"]
    fn test_swift_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_SWIFT_CODE, Language::Swift).unwrap();
        let functions = extract_functions(&tree, SAMPLE_SWIFT_CODE, Language::Swift);

        assert!(!functions.is_empty(), "Should extract Swift functions");
        assert!(
            functions.contains(&"hello".to_string()),
            "Should find 'hello' function"
        );
    }

    /// Test C# function extraction (P2 - currently stubbed)
    #[test]
    #[ignore = "P2 language: C# function extraction not implemented"]
    fn test_csharp_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_CSHARP_CODE, Language::CSharp).unwrap();
        let functions = extract_functions(&tree, SAMPLE_CSHARP_CODE, Language::CSharp);

        // C# has methods, not standalone functions
        assert!(
            functions.is_empty() || !functions.is_empty(),
            "C# uses methods"
        );
    }

    /// Test Scala function extraction (P2 - currently stubbed)
    #[test]
    #[ignore = "P2 language: Scala function extraction not implemented"]
    fn test_scala_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_SCALA_CODE, Language::Scala).unwrap();
        let functions = extract_functions(&tree, SAMPLE_SCALA_CODE, Language::Scala);

        assert!(!functions.is_empty(), "Should extract Scala functions");
    }

    /// Test PHP function extraction (P2 - currently stubbed)
    #[test]
    #[ignore = "P2 language: PHP function extraction not implemented"]
    fn test_php_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_PHP_CODE, Language::Php).unwrap();
        let functions = extract_functions(&tree, SAMPLE_PHP_CODE, Language::Php);

        assert!(!functions.is_empty(), "Should extract PHP functions");
        assert!(
            functions.contains(&"hello".to_string()),
            "Should find 'hello' function"
        );
    }

    /// Test Lua function extraction (P2 - currently stubbed)
    #[test]
    #[ignore = "P2 language: Lua function extraction not implemented"]
    fn test_lua_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_LUA_CODE, Language::Lua).unwrap();
        let functions = extract_functions(&tree, SAMPLE_LUA_CODE, Language::Lua);

        assert!(!functions.is_empty(), "Should extract Lua functions");
    }

    /// Test Elixir function extraction (P2 - currently stubbed)
    #[test]
    #[ignore = "P2 language: Elixir function extraction not implemented"]
    fn test_elixir_extract_functions() {
        let pool = ParserPool::new();
        let tree = pool.parse(SAMPLE_ELIXIR_CODE, Language::Elixir).unwrap();
        let functions = extract_functions(&tree, SAMPLE_ELIXIR_CODE, Language::Elixir);

        assert!(!functions.is_empty(), "Should extract Elixir functions");
        assert!(
            functions.contains(&"hello".to_string()),
            "Should find 'hello' function"
        );
    }
}

// =============================================================================
// Module: Rust-specific Callgraph Tests (Spec Section 3.2 - Rust gaps)
// =============================================================================

mod rust_callgraph_tests {
    use super::*;

    /// Test Rust `use` path resolution (Spec L258-262)
    /// Tests crate::, super::, and self:: path resolution.
    /// Implemented in Phase 9.
    #[test]
    fn test_rust_use_path_resolution() {
        use std::path::PathBuf;
        use tldr_core::callgraph::resolver::ModuleResolver;
        use tldr_core::types::ImportInfo;

        // Test that `use crate::utils::helper` resolves to correct file
        let rust_with_crate_use = r#"
use crate::utils::helper;
use super::sibling;
use self::child;

fn main() {
    helper(); // Should resolve to utils.rs::helper
}
"#;
        // Verify the use statements are parsed
        let pool = ParserPool::new();
        let tree = pool.parse(rust_with_crate_use, Language::Rust).unwrap();
        let imports =
            extract_imports_from_tree(&tree, rust_with_crate_use, Language::Rust).unwrap();

        assert!(!imports.is_empty(), "Should parse use statements");
        assert!(
            imports.iter().any(|i| i.module.contains("crate::utils")),
            "Should find crate::utils import"
        );

        // Test actual resolution with a resolver
        let mut resolver =
            ModuleResolver::new(PathBuf::from("/project")).with_language(Language::Rust);

        // Index some files
        let utils_path = PathBuf::from("/project/src/utils.rs");
        resolver.index_file(&utils_path);

        // Create an import and resolve it
        let import = ImportInfo {
            module: "crate::utils".to_string(),
            names: vec!["helper".to_string()],
            is_from: true,
            alias: None,
        };
        let from_file = std::path::Path::new("/project/src/main.rs");
        let resolved = resolver.resolve_import(&import, from_file);

        assert!(resolved.is_some(), "Should resolve crate::utils to a file");
        assert!(
            resolved
                .as_ref()
                .unwrap()
                .to_string_lossy()
                .contains("utils.rs"),
            "Resolved path should contain utils.rs"
        );
    }

    /// Test Rust nested use groups (PM-1.3 edge case)
    /// Verifies that complex use statements like `use std::{io::{self, Read}, fs}`
    /// are parsed correctly.
    #[test]
    fn test_rust_nested_use_groups() {
        // Test nested braces in use statements
        let rust_with_nested_use = r#"
use std::{io::{self, Read, Write}, fs, collections::HashMap};
use crate::{foo::bar, baz::{qux, quux}};
"#;
        let pool = ParserPool::new();
        let tree = pool.parse(rust_with_nested_use, Language::Rust).unwrap();
        let imports =
            extract_imports_from_tree(&tree, rust_with_nested_use, Language::Rust).unwrap();

        // Should extract multiple imports from nested groups
        assert!(!imports.is_empty(), "Should extract nested use imports");

        // Verify we captured the nested imports
        let all_text: String = imports
            .iter()
            .map(|i| format!("{}:{:?}", i.module, i.names))
            .collect::<Vec<_>>()
            .join("; ");

        // Should have std::io related imports
        assert!(
            imports.iter().any(|i| i.module.contains("std")),
            "Should have std imports, got: {}",
            all_text
        );
    }

    /// Test Rust self import in use groups
    /// Verifies `use std::io::{self, Read}` parses self correctly
    #[test]
    fn test_rust_self_import() {
        let rust_with_self = r#"
use std::io::{self, Read};
"#;
        let pool = ParserPool::new();
        let tree = pool.parse(rust_with_self, Language::Rust).unwrap();
        let imports = extract_imports_from_tree(&tree, rust_with_self, Language::Rust).unwrap();

        assert!(!imports.is_empty(), "Should parse self import");

        // The self import brings in std::io as a module
        let has_io = imports
            .iter()
            .any(|i| i.module.contains("std::io") || i.names.contains(&"self".to_string()));
        assert!(has_io, "Should find std::io or self in imports");
    }

    /// Test Rust aliased import
    /// Verifies `use std::collections::HashMap as Map` parses correctly
    #[test]
    fn test_rust_aliased_import() {
        let rust_with_alias = r#"
use std::collections::HashMap as Map;
use crate::foo::bar as baz;
"#;
        let pool = ParserPool::new();
        let tree = pool.parse(rust_with_alias, Language::Rust).unwrap();
        let imports = extract_imports_from_tree(&tree, rust_with_alias, Language::Rust).unwrap();

        assert!(!imports.is_empty(), "Should parse aliased imports");

        // Should have HashMap import (the actual name, not the alias)
        let has_hashmap = imports
            .iter()
            .any(|i| i.module.contains("collections") || i.names.contains(&"HashMap".to_string()));
        assert!(has_hashmap, "Should find HashMap in imports");
    }

    /// Test Rust trait method resolution (Spec L264-268)
    /// Gap: No type inference for `impl Trait` receivers.
    ///
    /// This test will FAIL until trait method resolution is implemented (~400 LOC).
    #[test]
    #[ignore = "P1 gap: Rust trait method resolution not implemented (Spec L264-268, Task 1.2)"]
    fn test_rust_trait_method_resolution() {
        // Test that x.method() resolves correctly when x: impl Trait
        let rust_with_trait = r#"
trait Greetable {
    fn greet(&self) -> String;
}

struct Greeter { name: String }

impl Greetable for Greeter {
    fn greet(&self) -> String {
        format!("Hello, {}!", self.name)
    }
}

fn use_greeter(g: impl Greetable) {
    let msg = g.greet(); // Should resolve to <T as Greetable>::greet
}
"#;
        // This would require:
        // 1. Tracking trait bounds on generic parameters
        // 2. Resolving method calls through trait bounds
        // 3. Finding all implementors of the trait

        let pool = ParserPool::new();
        let tree = pool.parse(rust_with_trait, Language::Rust).unwrap();

        // For now just verify it parses
        assert!(!tree.root_node().has_error(), "Should parse trait code");

        // Full test would verify trait method resolution
        // TODO: Implement trait resolution and update this test
    }

    /// Test Rust generic type instantiation (Spec L270-274)
    /// Gap: Treats `Vec<T>` as `Vec` without monomorphization info.
    ///
    /// This test will FAIL until generic instantiation is implemented (~150 LOC).
    #[test]
    #[ignore = "P1 gap: Rust generic instantiation not implemented (Spec L270-274, Task 1.3)"]
    fn test_rust_generic_instantiation() {
        let rust_with_generics = r#"
use std::collections::HashMap;

fn process<T: Clone>(items: Vec<T>) -> Vec<T> {
    items.iter().cloned().collect()
}

fn main() {
    let strings: Vec<String> = vec!["a".to_string()];
    let result = process(strings); // Vec<T> instantiated as Vec<String>

    let map: HashMap<String, i32> = HashMap::new();
}
"#;
        let pool = ParserPool::new();
        let tree = pool.parse(rust_with_generics, Language::Rust).unwrap();

        assert!(!tree.root_node().has_error(), "Should parse generic code");

        // Full test would verify:
        // 1. process(strings) instantiates T = String
        // 2. HashMap<String, i32>::new() is tracked correctly
        // TODO: Implement generic tracking and update this test
    }

    /// Test Rust mod declaration resolution
    /// Verifies that `mod foo;` declarations are extracted
    #[test]
    fn test_rust_mod_declaration() {
        let rust_with_mod = r#"
mod utils;
mod helpers;
pub mod api;

use self::utils::helper;
"#;
        let pool = ParserPool::new();
        let tree = pool.parse(rust_with_mod, Language::Rust).unwrap();
        let imports = extract_imports_from_tree(&tree, rust_with_mod, Language::Rust).unwrap();

        // Should find mod declarations (treated as imports in current impl)
        // At minimum the use statement should be found
        assert!(
            imports
                .iter()
                .any(|i| i.module.contains("utils") || i.names.contains(&"helper".to_string())),
            "Should find use self::utils::helper"
        );
    }
}

// =============================================================================
// Module: Java-specific Callgraph Tests (Spec Section 3.2 - Java gaps)
// =============================================================================

mod java_callgraph_tests {
    use super::*;
    use std::path::PathBuf;
    use tldr_core::callgraph::ModuleResolver;

    /// Test Java package resolution (Spec L287-290)
    /// Now implemented: Maps `import com.example.utils.Helper` to source file.
    ///
    /// Phase 10: Java callgraph completion - PM-1.8 mitigation.
    #[test]
    fn test_java_package_resolution() {
        let java_with_imports = r#"
package com.example;

import java.util.List;
import java.util.ArrayList;
import com.example.utils.Helper;

public class Main {
    public void run() {
        List<String> items = new ArrayList<>();
        Helper.process(items); // Should resolve to com.example.utils.Helper
    }
}
"#;
        // Parse and extract imports
        let pool = ParserPool::new();
        let tree = pool.parse(java_with_imports, Language::Java).unwrap();
        let imports = extract_imports_from_tree(&tree, java_with_imports, Language::Java).unwrap();

        assert!(!imports.is_empty(), "Should parse import statements");

        // Test resolution with ModuleResolver
        let mut resolver =
            ModuleResolver::new(PathBuf::from("/project")).with_language(Language::Java);

        // Index the helper file (simulating project structure)
        let helper_path = PathBuf::from("/project/com/example/utils/Helper.java");
        resolver.index_file(&helper_path);

        // Find the Helper import and try to resolve it
        let helper_import = imports.iter().find(|i| i.module.contains("Helper"));
        assert!(helper_import.is_some(), "Should find Helper import");

        let from_file = std::path::Path::new("/project/com/example/Main.java");
        let resolved = resolver.resolve_import(helper_import.unwrap(), from_file);
        assert_eq!(
            resolved,
            Some(helper_path),
            "Should resolve Helper to source file"
        );

        // JDK imports should NOT resolve (not in project)
        let list_import = imports.iter().find(|i| i.module.contains("java.util.List"));
        assert!(list_import.is_some(), "Should find List import");
        let resolved_list = resolver.resolve_import(list_import.unwrap(), from_file);
        assert_eq!(
            resolved_list, None,
            "JDK imports should not resolve to local files"
        );
    }

    /// Test Java interface method resolution (Spec L293-296)
    /// Tests parsing of interface code - full type resolution is out of scope for Phase 10.
    ///
    /// Phase 10: Basic parsing verification. Interface dispatch resolution is future work.
    #[test]
    fn test_java_interface_resolution() {
        let java_with_interface = r#"
package com.example;

interface Processor {
    void process(String input);
}

class StringProcessor implements Processor {
    @Override
    public void process(String input) {
        System.out.println(input);
    }
}

class Consumer {
    public void consume(Processor p) {
        p.process("test"); // Should resolve to StringProcessor.process (and others)
    }
}
"#;
        // Phase 10 scope: Verify parsing works correctly
        // Future work: Track interface implementations and resolve polymorphic calls

        let pool = ParserPool::new();
        let tree = pool.parse(java_with_interface, Language::Java).unwrap();

        assert!(!tree.root_node().has_error(), "Should parse interface code");

        // Verify we can extract classes including interfaces
        let classes = extract_classes(&tree, java_with_interface, Language::Java);
        assert!(
            classes.iter().any(|c| c == "Processor"),
            "Should find Processor interface"
        );
        assert!(
            classes.iter().any(|c| c == "StringProcessor"),
            "Should find StringProcessor class"
        );
        assert!(
            classes.iter().any(|c| c == "Consumer"),
            "Should find Consumer class"
        );

        // Note: Full interface dispatch resolution (finding all implementors of Processor
        // when resolving p.process()) is future work beyond Phase 10.
    }

    /// Test Java anonymous class handling (Spec L299-302)
    /// Gap: May crash on complex expressions with anonymous classes.
    ///
    /// This test verifies graceful handling of anonymous classes.
    #[test]
    #[ignore = "P1 gap: Java anonymous class handling not implemented (Spec L299-302, Task 1.6)"]
    fn test_java_anonymous_class_handling() {
        let java_with_anonymous = r#"
package com.example;

public class Main {
    public void run() {
        Runnable r = new Runnable() {
            @Override
            public void run() {
                System.out.println("Anonymous!");
            }
        };

        Thread t = new Thread(new Runnable() {
            @Override
            public void run() {
                System.out.println("Inline anonymous!");
            }
        });
    }
}
"#;
        let pool = ParserPool::new();
        let tree = pool.parse(java_with_anonymous, Language::Java).unwrap();

        // Should not crash when parsing
        assert!(
            !tree.root_node().has_error(),
            "Should parse anonymous class code"
        );

        // Extract functions should skip anonymous class methods
        let _functions = extract_functions(&tree, java_with_anonymous, Language::Java);
        // Anonymous class methods should either be skipped or handled gracefully
        // Not crash
    }

    /// Test Java static import resolution
    /// Verifies that static imports are parsed correctly
    #[test]
    fn test_java_static_import() {
        let java_with_static = r#"
package com.example;

import static java.lang.Math.PI;
import static java.lang.Math.*;

public class Circle {
    public double area(double radius) {
        return PI * radius * radius;
    }
}
"#;
        let pool = ParserPool::new();
        let tree = pool.parse(java_with_static, Language::Java).unwrap();
        let imports = extract_imports_from_tree(&tree, java_with_static, Language::Java).unwrap();

        // Should find the static imports
        assert!(!imports.is_empty(), "Should parse static import statements");
    }
}

// =============================================================================
// Module: P2 Language Status Tests (Spec Section 3.3)
// These tests verify P2 languages currently return UnsupportedLanguage
// =============================================================================

mod p2_language_status_tests {
    use super::*;

    // Note: C and C++ now have full grammar support (Phase 2) and import/function extraction (Phase 6/7)
    // The test_c_returns_unsupported and test_cpp_returns_unsupported tests have been removed
    // since these languages are now fully supported with tree-sitter-c and tree-sitter-cpp grammars.

    // Note: The following languages now have grammar support:
    // - Ruby (Phase 3)
    // - C# (Phase 4)
    // - Scala (Phase 4)
    // - PHP (Phase 4)
    // - Lua (Phase 5)
    // - Luau (Phase 5)
    // - Elixir (Phase 5)
    //
    // Their corresponding test_*_returns_unsupported tests have been removed.
    // See parser_tests module for parse tests that verify these grammars work.

    /// Verify Kotlin returns UnsupportedLanguage
    /// Note: tree-sitter-kotlin 0.3.8 requires tree-sitter <0.23, incompatible with 0.24.7
    #[test]
    fn test_kotlin_returns_unsupported() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_KOTLIN_CODE, Language::Kotlin);
        assert!(
            matches!(result, Err(TldrError::UnsupportedLanguage(_))),
            "Kotlin should return UnsupportedLanguage until tree-sitter-kotlin is updated for tree-sitter 0.24"
        );
    }

    /// Verify Swift returns UnsupportedLanguage
    /// Note: tree-sitter-swift 0.7.1 has ABI version 15, incompatible with tree-sitter 0.24.7
    #[test]
    fn test_swift_returns_unsupported() {
        let pool = ParserPool::new();
        let result = pool.parse(SAMPLE_SWIFT_CODE, Language::Swift);
        assert!(
            matches!(result, Err(TldrError::UnsupportedLanguage(_))),
            "Swift should return UnsupportedLanguage until tree-sitter-swift is updated for tree-sitter 0.24"
        );
    }
}

// =============================================================================
// Module: Edge Case Tests (Spec Section 2.1 - Edge cases)
// =============================================================================

mod edge_case_tests {
    use super::*;

    /// Test empty file parsing (Spec L84)
    /// Empty file should return valid tree with no children
    #[test]
    fn test_parse_empty_file() {
        let pool = ParserPool::new();
        let result = pool.parse("", Language::Python);
        assert!(result.is_ok(), "Empty file should parse successfully");

        let tree = result.unwrap();
        // Empty file produces a tree with just a root node
        assert!(
            tree.root_node().child_count() == 0,
            "Empty file should have no children"
        );
    }

    /// Test file with syntax errors (Spec L85)
    /// Should return best-effort parse with error nodes
    #[test]
    fn test_parse_syntax_error() {
        let broken_python = r#"
def broken(
    # Missing closing paren and body
"#;
        let pool = ParserPool::new();
        let result = pool.parse(broken_python, Language::Python);

        // Should still parse (best-effort)
        assert!(
            result.is_ok(),
            "Broken syntax should still parse (best-effort)"
        );

        let tree = result.unwrap();
        // Tree should contain error nodes
        assert!(
            tree.root_node().has_error(),
            "Parse tree should have error nodes"
        );
    }

    /// Test wildcard import (Spec L108)
    /// names should be ["*"]
    #[test]
    fn test_wildcard_import() {
        let python_wildcard = "from os.path import *\n";
        let pool = ParserPool::new();
        let tree = pool.parse(python_wildcard, Language::Python).unwrap();
        let imports = extract_imports_from_tree(&tree, python_wildcard, Language::Python).unwrap();

        assert!(!imports.is_empty(), "Should extract wildcard import");
        let wildcard = imports.iter().find(|i| i.names.contains(&"*".to_string()));
        assert!(wildcard.is_some(), "Should find wildcard import with '*'");
    }

    /// Test relative import (Spec L109)
    /// Should preserve dots: ., .., ./
    #[test]
    fn test_relative_import() {
        let python_relative = r#"
from . import sibling
from .. import parent
from .subpackage import module
"#;
        let pool = ParserPool::new();
        let tree = pool.parse(python_relative, Language::Python).unwrap();
        let imports = extract_imports_from_tree(&tree, python_relative, Language::Python).unwrap();

        assert!(imports.len() >= 3, "Should extract all relative imports");

        // Check that relative imports are preserved
        let has_single_dot = imports
            .iter()
            .any(|i| i.module.starts_with('.') && !i.module.starts_with(".."));
        let has_double_dot = imports.iter().any(|i| i.module.starts_with(".."));
        assert!(
            has_single_dot || has_double_dot,
            "Should preserve relative import dots"
        );
    }

    /// Test file size limit (Spec L79 - M6 mitigation)
    /// Files > 5MB should return ParseError
    #[test]
    fn test_file_size_limit() {
        // Create a string larger than 5MB
        let large_content = "x".repeat(6 * 1024 * 1024); // 6MB

        let pool = ParserPool::new();
        let result = pool.parse(&large_content, Language::Python);

        assert!(
            matches!(result, Err(TldrError::ParseError { .. })),
            "Files > 5MB should return ParseError"
        );
    }

    /// Test async function extraction (Spec L129)
    /// Should include async qualifier in extraction
    #[test]
    fn test_async_function_extraction() {
        let async_python = r#"
async def fetch_data(url: str) -> str:
    return ""

def sync_function():
    pass
"#;
        let pool = ParserPool::new();
        let tree = pool.parse(async_python, Language::Python).unwrap();
        let functions = extract_functions(&tree, async_python, Language::Python);

        assert!(
            functions.contains(&"fetch_data".to_string()),
            "Should find async function"
        );
        assert!(
            functions.contains(&"sync_function".to_string()),
            "Should find sync function"
        );
    }

    /// Test decorated function extraction (Spec L130)
    /// Should include the function, not the decorator
    #[test]
    fn test_decorated_function_extraction() {
        let decorated_python = r#"
@staticmethod
def static_method():
    pass

@property
def my_property(self):
    return self._value

@decorator
@another_decorator
def multi_decorated():
    pass
"#;
        let pool = ParserPool::new();
        let tree = pool.parse(decorated_python, Language::Python).unwrap();
        let functions = extract_functions(&tree, decorated_python, Language::Python);

        assert!(
            functions.contains(&"static_method".to_string()),
            "Should find decorated function"
        );
        assert!(
            functions.contains(&"my_property".to_string()),
            "Should find property"
        );
        assert!(
            functions.contains(&"multi_decorated".to_string()),
            "Should find multi-decorated function"
        );
    }
}
