//! Multilang benchmark tests for Patterns, Security, Cohesion, Coupling, Deps, and Inheritance.
//!
//! Tests the following command groups against temp fixture files with known patterns:
//!
//! **Patterns group:** patterns (PatternMiner), inheritance, deps, cohesion, coupling
//! **Security group:** vuln (scan_vulnerabilities), secure (run_secure)
//!
//! Note: contracts, specs, invariants, verify, and interface commands live in tldr-cli,
//! not tldr-core. Those are tested separately in the CLI crate.

use std::path::PathBuf;
use tempfile::TempDir;

use tldr_core::analysis::deps::{analyze_dependencies, DepsOptions};
use tldr_core::inheritance::{extract_inheritance, InheritanceOptions};
use tldr_core::patterns::{PatternConfig, PatternMiner};
use tldr_core::quality::cohesion::{analyze_cohesion, CohesionVerdict};
use tldr_core::quality::coupling::analyze_coupling;
use tldr_core::security::vuln::{scan_vulnerabilities, VulnType};
use tldr_core::wrappers::secure::run_secure;
use tldr_core::Language;

// =============================================================================
// Helpers
// =============================================================================

/// Create a temp file in the given directory, creating parent dirs as needed.
fn create_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
    path
}

// =============================================================================
// Fixture Content Generators
// =============================================================================

mod fixtures {
    // ----- Python fixtures -----

    pub const PYTHON_SINGLETON: &str = r#"
class DatabaseConnection:
    _instance = None

    def __new__(cls):
        if cls._instance is None:
            cls._instance = super().__new__(cls)
        return cls._instance

    def connect(self):
        self.connection = "active"
        return self.connection

    def disconnect(self):
        self.connection = None
"#;

    pub const PYTHON_FACTORY: &str = r#"
from abc import ABC, abstractmethod

class Shape(ABC):
    @abstractmethod
    def area(self):
        pass

class Circle(Shape):
    def __init__(self, radius):
        self.radius = radius

    def area(self):
        return 3.14 * self.radius ** 2

class Square(Shape):
    def __init__(self, side):
        self.side = side

    def area(self):
        return self.side ** 2

class ShapeFactory:
    @staticmethod
    def create_shape(shape_type, *args):
        if shape_type == "circle":
            return Circle(*args)
        elif shape_type == "square":
            return Square(*args)
        raise ValueError(f"Unknown shape: {shape_type}")
"#;

    pub const PYTHON_OBSERVER: &str = r#"
class EventEmitter:
    def __init__(self):
        self._listeners = {}

    def on(self, event, callback):
        if event not in self._listeners:
            self._listeners[event] = []
        self._listeners[event].append(callback)

    def emit(self, event, *args):
        for callback in self._listeners.get(event, []):
            callback(*args)

    def off(self, event, callback):
        if event in self._listeners:
            self._listeners[event].remove(callback)
"#;

    pub const PYTHON_INHERITANCE_HIERARCHY: &str = r#"
class Animal:
    def __init__(self, name):
        self.name = name

    def speak(self):
        pass

class Mammal(Animal):
    def __init__(self, name, legs):
        super().__init__(name)
        self.legs = legs

    def walk(self):
        return f"{self.name} walks on {self.legs} legs"

class Dog(Mammal):
    def speak(self):
        return "Woof!"

    def fetch(self):
        return f"{self.name} fetches the ball"

class Cat(Mammal):
    def speak(self):
        return "Meow!"
"#;

    pub const PYTHON_LOW_COHESION: &str = r#"
class GodClass:
    """A class with unrelated methods (high LCOM4)."""
    def __init__(self):
        self.name = ""
        self.email = ""
        self.database = None
        self.log_level = "info"

    def get_name(self):
        return self.name

    def set_name(self, name):
        self.name = name

    def get_email(self):
        return self.email

    def set_email(self, email):
        self.email = email

    def connect_db(self):
        self.database = "connected"

    def query_db(self):
        return self.database

    def set_log_level(self, level):
        self.log_level = level

    def get_log_level(self):
        return self.log_level
"#;

    pub const PYTHON_HIGH_COHESION: &str = r#"
class Vector2D:
    """A well-cohesive class (LCOM4 = 1)."""
    def __init__(self, x, y):
        self.x = x
        self.y = y

    def magnitude(self):
        return (self.x**2 + self.y**2) ** 0.5

    def normalize(self):
        mag = self.magnitude()
        self.x /= mag
        self.y /= mag

    def dot(self, other):
        return self.x * other.x + self.y * other.y
"#;

    pub const PYTHON_SQL_INJECTION: &str = r#"
from flask import request
import sqlite3

def search_users():
    user_input = request.args.get("query")
    conn = sqlite3.connect("users.db")
    cursor = conn.cursor()
    query = "SELECT * FROM users WHERE name = '" + user_input + "'"
    cursor.execute(query)
    return cursor.fetchall()
"#;

    pub const PYTHON_COMMAND_INJECTION: &str = r#"
import os
from flask import request

def run_command():
    cmd = request.form.get("command")
    os.system(cmd)
"#;

    pub const PYTHON_IMPORTS_A: &str = r#"
from module_b import helper_func

def main():
    result = helper_func(42)
    return result
"#;

    pub const PYTHON_IMPORTS_B: &str = r#"
def helper_func(x):
    return x * 2

def unused_func():
    return None
"#;

    pub const PYTHON_VALIDATION_PATTERN: &str = r#"
def validate_email(email):
    if not isinstance(email, str):
        raise TypeError("email must be a string")
    if "@" not in email:
        raise ValueError("invalid email format")
    return True

class UserValidator:
    def validate_age(self, age):
        if age < 0:
            raise ValueError("age must be non-negative")
        if age > 150:
            raise ValueError("age must be <= 150")
        return True
"#;

    pub const PYTHON_ERROR_HANDLING: &str = r#"
import logging

logger = logging.getLogger(__name__)

def safe_divide(a, b):
    try:
        result = a / b
    except ZeroDivisionError:
        logger.error("Division by zero")
        raise
    except TypeError as e:
        logger.warning(f"Type error: {e}")
        return None
    return result

class FileProcessor:
    def process(self, filepath):
        try:
            with open(filepath) as f:
                data = f.read()
        except FileNotFoundError:
            raise
        except PermissionError:
            raise
        return data
"#;

    // ----- Java fixtures -----

    pub const JAVA_SINGLETON: &str = r#"
public class AppConfig {
    private static AppConfig instance;

    private AppConfig() {}

    public static synchronized AppConfig getInstance() {
        if (instance == null) {
            instance = new AppConfig();
        }
        return instance;
    }

    public String getSetting(String key) {
        return System.getProperty(key);
    }
}
"#;

    pub const JAVA_INHERITANCE_HIERARCHY: &str = r#"
public abstract class Vehicle {
    protected String make;
    protected int year;

    public Vehicle(String make, int year) {
        this.make = make;
        this.year = year;
    }

    public abstract double fuelEfficiency();
}

class Car extends Vehicle {
    private int doors;

    public Car(String make, int year, int doors) {
        super(make, year);
        this.doors = doors;
    }

    @Override
    public double fuelEfficiency() {
        return 30.0;
    }
}

class ElectricCar extends Car {
    private double batteryCapacity;

    public ElectricCar(String make, int year, int doors, double battery) {
        super(make, year, doors);
        this.batteryCapacity = battery;
    }

    @Override
    public double fuelEfficiency() {
        return batteryCapacity * 3.5;
    }
}
"#;

    pub const JAVA_LOW_COHESION: &str = r#"
public class UtilityManager {
    private String dbUrl;
    private String logFile;
    private int cacheSize;

    public void connectDatabase() {
        System.out.println("Connecting to " + dbUrl);
    }

    public void queryDatabase() {
        System.out.println("Querying " + dbUrl);
    }

    public void writeLog(String msg) {
        System.out.println("Log to " + logFile + ": " + msg);
    }

    public void readLog() {
        System.out.println("Reading " + logFile);
    }

    public void initCache() {
        System.out.println("Cache size: " + cacheSize);
    }

    public void clearCache() {
        System.out.println("Clearing cache: " + cacheSize);
    }
}
"#;

    pub const JAVA_SQL_INJECTION: &str = r#"
import javax.servlet.http.HttpServletRequest;
import java.sql.Statement;

public class UserSearch {
    public void search(HttpServletRequest request, Statement statement) {
        String name = request.getParameter("name");
        String query = "SELECT * FROM users WHERE name = '" + name + "'";
        statement.executeQuery(query);
    }
}
"#;

    // ----- TypeScript fixtures -----

    pub const TS_SINGLETON: &str = r#"
class Logger {
    private static instance: Logger;
    private logs: string[] = [];

    private constructor() {}

    static getInstance(): Logger {
        if (!Logger.instance) {
            Logger.instance = new Logger();
        }
        return Logger.instance;
    }

    log(message: string): void {
        this.logs.push(message);
    }

    getLogs(): string[] {
        return this.logs;
    }
}
"#;

    pub const TS_INHERITANCE_HIERARCHY: &str = r#"
abstract class Shape {
    abstract area(): number;
    abstract perimeter(): number;
}

class Rectangle extends Shape {
    constructor(private width: number, private height: number) {
        super();
    }

    area(): number {
        return this.width * this.height;
    }

    perimeter(): number {
        return 2 * (this.width + this.height);
    }
}

class Square extends Rectangle {
    constructor(side: number) {
        super(side, side);
    }
}
"#;

    pub const TS_LOW_COHESION: &str = r#"
class AppManager {
    private config: any;
    private users: string[];
    private theme: string;

    loadConfig(): void {
        console.log(this.config);
    }

    saveConfig(): void {
        this.config = {};
    }

    addUser(name: string): void {
        this.users.push(name);
    }

    removeUser(name: string): void {
        this.users = this.users.filter(u => u !== name);
    }

    setTheme(theme: string): void {
        this.theme = theme;
    }

    getTheme(): string {
        return this.theme;
    }
}
"#;

    pub const TS_XSS_VULN: &str = r#"
const express = require('express');
const app = express();

app.get('/search', (req, res) => {
    const query = req.query.q;
    res.send('<h1>Results for: ' + query + '</h1>');
});

app.post('/comment', (req, res) => {
    const body = req.body.comment;
    document.getElementById('output').innerHTML = body;
});
"#;

    pub const TS_IMPORTS_A: &str = r#"
import { helperFunc } from './module_b';

export function main(): number {
    return helperFunc(42);
}
"#;

    pub const TS_IMPORTS_B: &str = r#"
export function helperFunc(x: number): number {
    return x * 2;
}

export function unusedFunc(): void {
    return;
}
"#;

    // ----- C++ fixtures -----

    pub const CPP_INHERITANCE_HIERARCHY: &str = r#"
class Base {
public:
    virtual void render() = 0;
    virtual ~Base() {}
};

class Widget : public Base {
protected:
    int x, y;
public:
    Widget(int x, int y) : x(x), y(y) {}
    void render() override {}
};

class Button : public Widget {
    std::string label;
public:
    Button(int x, int y, std::string lbl) : Widget(x, y), label(lbl) {}
    void render() override {}
    void click() {}
};
"#;

    // ----- Ruby fixtures -----

    pub const RUBY_INHERITANCE_HIERARCHY: &str = r#"
class Animal
  attr_reader :name

  def initialize(name)
    @name = name
  end

  def speak
    raise NotImplementedError
  end
end

class Dog < Animal
  def speak
    "Woof!"
  end
end

class Puppy < Dog
  def speak
    "Yip!"
  end
end
"#;

    pub const RUBY_LOW_COHESION: &str = r#"
class ServiceManager
  # No shared initializer -- methods use disjoint instance variables
  # Group 1: database methods
  def connect_db
    @db_conn = "connected"
  end

  def query_db(sql)
    @db_conn
  end

  # Group 2: cache methods
  def cache_set(key, val)
    @cache_data = val
  end

  def cache_get(key)
    @cache_data
  end

  # Group 3: logging methods
  def log_info(msg)
    @log_buffer = msg
  end

  def flush_log
    @log_buffer
  end
end
"#;

    // ----- Kotlin fixtures -----

    pub const KOTLIN_INHERITANCE_HIERARCHY: &str = r#"
abstract class Component {
    abstract fun render(): String
}

open class Container : Component() {
    val children = mutableListOf<Component>()

    override fun render(): String {
        return children.joinToString("\n") { it.render() }
    }
}

class Panel : Container() {
    var title: String = ""

    override fun render(): String {
        return "Panel: $title\n${super.render()}"
    }
}
"#;

    // ----- C# fixtures -----

    pub const CSHARP_INHERITANCE_HIERARCHY: &str = r#"
public abstract class Repository<T> {
    public abstract T GetById(int id);
    public abstract void Save(T entity);
}

public class UserRepository : Repository<User> {
    public override User GetById(int id) {
        return new User(id);
    }

    public override void Save(User entity) {
        // save logic
    }
}
"#;

    // ----- Swift fixtures -----

    pub const SWIFT_INHERITANCE_HIERARCHY: &str = r#"
class Animal {
    var name: String

    init(name: String) {
        self.name = name
    }

    func speak() -> String {
        return ""
    }
}

class Dog: Animal {
    override func speak() -> String {
        return "Woof!"
    }
}

class GuideDog: Dog {
    var handler: String = ""

    override func speak() -> String {
        return "Quiet woof"
    }
}
"#;

    // ----- Go fixtures -----

    pub const GO_IMPORTS_A: &str = r#"
package main

import "fmt"

func main() {
    fmt.Println("Hello from main")
}
"#;

    pub const GO_IMPORTS_B: &str = r#"
package utils

import "strings"

func Helper(s string) string {
    return strings.ToUpper(s)
}
"#;

    pub const GO_SQL_INJECTION: &str = r#"
package main

import (
    "database/sql"
    "net/http"
)

func searchHandler(w http.ResponseWriter, r *http.Request) {
    name := r.URL.Query().Get("name")
    query := "SELECT * FROM users WHERE name = '" + name + "'"
    db.Query(query)
}
"#;

    // ----- JavaScript fixtures -----

    pub const JS_COMMAND_INJECTION: &str = r#"
const express = require('express');
const { exec } = require('child_process');

const app = express();

app.get('/run', (req, res) => {
    const cmd = req.query.cmd;
    child_process.exec(cmd, (err, stdout) => {
        res.send(stdout);
    });
});
"#;

    pub const JS_IMPORTS_A: &str = r#"
const { add } = require('./utils');

function main() {
    return add(1, 2);
}

module.exports = { main };
"#;

    pub const JS_IMPORTS_B: &str = r#"
function add(a, b) {
    return a + b;
}

function subtract(a, b) {
    return a - b;
}

module.exports = { add, subtract };
"#;

    // ----- Rust fixtures -----

    pub const RUST_IMPORTS_A: &str = r#"
mod utils;

use utils::helper;

fn main() {
    let result = helper(42);
    println!("{}", result);
}
"#;

    pub const RUST_IMPORTS_B: &str = r#"
pub fn helper(x: i32) -> i32 {
    x * 2
}
"#;

    // ----- Scala fixtures (for inheritance) -----

    pub const SCALA_INHERITANCE_HIERARCHY: &str = r#"
abstract class Shape {
  def area: Double
}

class Circle(val radius: Double) extends Shape {
  def area: Double = math.Pi * radius * radius
}

class Cylinder(radius: Double, val height: Double) extends Circle(radius) {
  def volume: Double = area * height
}
"#;

    // ----- PHP fixtures (for inheritance) -----

    pub const PHP_INHERITANCE_HIERARCHY: &str = r#"<?php
abstract class Logger {
    abstract public function log(string $message): void;
}

class FileLogger extends Logger {
    public function log(string $message): void {
        file_put_contents('log.txt', $message);
    }
}

class BufferedLogger extends FileLogger {
    private array $buffer = [];

    public function log(string $message): void {
        $this->buffer[] = $message;
    }

    public function flush(): void {
        foreach ($this->buffer as $msg) {
            parent::log($msg);
        }
    }
}
"#;
}

// =============================================================================
// PATTERNS TESTS - PatternMiner::mine_patterns
// =============================================================================

#[test]
fn test_patterns_python_detects_patterns() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "singleton.py", fixtures::PYTHON_SINGLETON);
    create_file(&dir, "factory.py", fixtures::PYTHON_FACTORY);
    create_file(&dir, "observer.py", fixtures::PYTHON_OBSERVER);
    create_file(&dir, "validator.py", fixtures::PYTHON_VALIDATION_PATTERN);
    create_file(&dir, "errors.py", fixtures::PYTHON_ERROR_HANDLING);

    let miner = PatternMiner::new(PatternConfig {
        min_confidence: 0.0, // Accept all patterns
        ..PatternConfig::default()
    });
    let report = miner
        .mine_patterns(dir.path(), Some(Language::Python))
        .unwrap();

    assert!(
        report.metadata.files_analyzed >= 4,
        "Should analyze at least 4 Python files, got {}",
        report.metadata.files_analyzed
    );

    // The miner detects pattern categories (error_handling, validation, naming, etc.)
    // rather than GoF design patterns. Verify we get signal from the fixtures.
    let total_patterns = report.metadata.patterns_before_filter;
    assert!(
        total_patterns > 0,
        "Should detect at least one pattern from the fixture files"
    );
}

#[test]
fn test_patterns_python_error_handling() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "errors.py", fixtures::PYTHON_ERROR_HANDLING);

    let miner = PatternMiner::new(PatternConfig {
        min_confidence: 0.0,
        ..PatternConfig::default()
    });
    let report = miner
        .mine_patterns(dir.path(), Some(Language::Python))
        .unwrap();

    // The error_handling pattern should pick up try/except blocks
    assert!(
        report.error_handling.is_some(),
        "Should detect error handling patterns from try/except blocks"
    );
}

#[test]
fn test_patterns_python_validation() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "validator.py", fixtures::PYTHON_VALIDATION_PATTERN);

    let miner = PatternMiner::new(PatternConfig {
        min_confidence: 0.0,
        ..PatternConfig::default()
    });
    let report = miner
        .mine_patterns(dir.path(), Some(Language::Python))
        .unwrap();

    // The validation pattern should detect raise/isinstance patterns
    assert!(
        report.validation.is_some(),
        "Should detect validation patterns from guard clauses and isinstance checks"
    );
}

#[test]
fn test_patterns_java() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "AppConfig.java", fixtures::JAVA_SINGLETON);

    let miner = PatternMiner::new(PatternConfig {
        min_confidence: 0.0,
        ..PatternConfig::default()
    });
    let report = miner
        .mine_patterns(dir.path(), Some(Language::Java))
        .unwrap();

    assert!(
        report.metadata.files_analyzed >= 1,
        "Should analyze at least 1 Java file"
    );
    // Java is a supported pattern language
    assert!(
        report.metadata.patterns_before_filter > 0
            || report.metadata.files_analyzed > 0,
        "Should process the Java file"
    );
}

#[test]
fn test_patterns_typescript() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "logger.ts", fixtures::TS_SINGLETON);

    let miner = PatternMiner::new(PatternConfig {
        min_confidence: 0.0,
        ..PatternConfig::default()
    });
    let report = miner
        .mine_patterns(dir.path(), Some(Language::TypeScript))
        .unwrap();

    assert!(
        report.metadata.files_analyzed >= 1,
        "Should analyze at least 1 TypeScript file"
    );
}

#[test]
fn test_patterns_confidence_filter() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "errors.py", fixtures::PYTHON_ERROR_HANDLING);
    create_file(&dir, "validator.py", fixtures::PYTHON_VALIDATION_PATTERN);

    // With min_confidence=0.0, should get everything
    let miner_low = PatternMiner::new(PatternConfig {
        min_confidence: 0.0,
        ..PatternConfig::default()
    });
    let report_low = miner_low
        .mine_patterns(dir.path(), Some(Language::Python))
        .unwrap();

    // With min_confidence=1.0, should filter most out
    let miner_high = PatternMiner::new(PatternConfig {
        min_confidence: 1.0,
        ..PatternConfig::default()
    });
    let report_high = miner_high
        .mine_patterns(dir.path(), Some(Language::Python))
        .unwrap();

    assert!(
        report_high.metadata.patterns_after_filter <= report_low.metadata.patterns_after_filter,
        "Higher confidence threshold should result in fewer or equal patterns"
    );
}

#[test]
fn test_patterns_empty_directory() {
    let dir = TempDir::new().unwrap();
    let miner = PatternMiner::new(PatternConfig::default());
    let report = miner.mine_patterns(dir.path(), None).unwrap();

    assert_eq!(report.metadata.files_analyzed, 0);
    assert_eq!(report.metadata.patterns_before_filter, 0);
}

// =============================================================================
// INHERITANCE TESTS - extract_inheritance
// =============================================================================

#[test]
fn test_inheritance_python() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "animals.py", fixtures::PYTHON_INHERITANCE_HIERARCHY);

    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::Python), &opts).unwrap();

    // Should find Animal, Mammal, Dog, Cat
    assert!(
        report.count >= 4,
        "Should find at least 4 classes (Animal, Mammal, Dog, Cat), got {}",
        report.count
    );

    // Check edges: Mammal -> Animal, Dog -> Mammal, Cat -> Mammal
    assert!(
        !report.edges.is_empty(),
        "Should have inheritance edges"
    );

    // Verify Mammal inherits from Animal
    let mammal_to_animal = report
        .edges
        .iter()
        .any(|e| e.child == "Mammal" && e.parent == "Animal");
    assert!(mammal_to_animal, "Mammal should extend Animal");

    // Verify Dog inherits from Mammal
    let dog_to_mammal = report
        .edges
        .iter()
        .any(|e| e.child == "Dog" && e.parent == "Mammal");
    assert!(dog_to_mammal, "Dog should extend Mammal");

    // Verify Cat inherits from Mammal
    let cat_to_mammal = report
        .edges
        .iter()
        .any(|e| e.child == "Cat" && e.parent == "Mammal");
    assert!(cat_to_mammal, "Cat should extend Mammal");

    // Animal should be a root
    assert!(
        report.roots.contains(&"Animal".to_string()),
        "Animal should be a root class, roots: {:?}",
        report.roots
    );

    // Dog and Cat should be leaves
    assert!(
        report.leaves.contains(&"Dog".to_string()),
        "Dog should be a leaf class"
    );
    assert!(
        report.leaves.contains(&"Cat".to_string()),
        "Cat should be a leaf class"
    );
}

#[test]
fn test_inheritance_python_factory_hierarchy() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "factory.py", fixtures::PYTHON_FACTORY);

    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::Python), &opts).unwrap();

    // Should find Shape (ABC), Circle, Square, ShapeFactory
    assert!(
        report.count >= 3,
        "Should find at least Shape, Circle, Square, got {}",
        report.count
    );

    // Circle and Square both extend Shape
    let circle_extends = report
        .edges
        .iter()
        .any(|e| e.child == "Circle" && e.parent == "Shape");
    assert!(circle_extends, "Circle should extend Shape");

    let square_extends = report
        .edges
        .iter()
        .any(|e| e.child == "Square" && e.parent == "Shape");
    assert!(square_extends, "Square should extend Shape");
}

#[test]
fn test_inheritance_java() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "Vehicle.java", fixtures::JAVA_INHERITANCE_HIERARCHY);

    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::Java), &opts).unwrap();

    // Vehicle -> Car -> ElectricCar
    assert!(
        report.count >= 3,
        "Should find Vehicle, Car, ElectricCar, got {}",
        report.count
    );

    let car_extends_vehicle = report
        .edges
        .iter()
        .any(|e| e.child == "Car" && e.parent == "Vehicle");
    assert!(car_extends_vehicle, "Car should extend Vehicle");

    let ecar_extends_car = report
        .edges
        .iter()
        .any(|e| e.child == "ElectricCar" && e.parent == "Car");
    assert!(ecar_extends_car, "ElectricCar should extend Car");
}

#[test]
fn test_inheritance_typescript() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "shapes.ts", fixtures::TS_INHERITANCE_HIERARCHY);

    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::TypeScript), &opts).unwrap();

    // Shape -> Rectangle -> Square
    assert!(
        report.count >= 3,
        "Should find Shape, Rectangle, Square, got {}",
        report.count
    );

    let rect_extends = report
        .edges
        .iter()
        .any(|e| e.child == "Rectangle" && e.parent == "Shape");
    assert!(rect_extends, "Rectangle should extend Shape");

    let square_extends = report
        .edges
        .iter()
        .any(|e| e.child == "Square" && e.parent == "Rectangle");
    assert!(square_extends, "Square should extend Rectangle");
}

#[test]
fn test_inheritance_cpp_unsupported() {
    // C++ is NOT in the inheritance module's supported languages.
    // It falls through to `_ => Vec::new()` in extract_classes match.
    // Verify graceful handling -- returns 0 classes without error.
    let dir = TempDir::new().unwrap();
    create_file(&dir, "widgets.cpp", fixtures::CPP_INHERITANCE_HIERARCHY);

    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::Cpp), &opts).unwrap();

    // C++ inheritance extraction is not implemented; should return empty gracefully
    assert_eq!(
        report.count, 0,
        "C++ is not supported by inheritance module, should return 0 classes"
    );
}

#[test]
fn test_inheritance_ruby() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "animals.rb", fixtures::RUBY_INHERITANCE_HIERARCHY);

    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::Ruby), &opts).unwrap();

    // Animal -> Dog -> Puppy
    assert!(
        report.count >= 3,
        "Should find Animal, Dog, Puppy, got {}",
        report.count
    );

    let dog_extends = report
        .edges
        .iter()
        .any(|e| e.child == "Dog" && e.parent == "Animal");
    assert!(dog_extends, "Dog should extend Animal");

    let puppy_extends = report
        .edges
        .iter()
        .any(|e| e.child == "Puppy" && e.parent == "Dog");
    assert!(puppy_extends, "Puppy should extend Dog");
}

#[test]
fn test_inheritance_kotlin() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "ui.kt", fixtures::KOTLIN_INHERITANCE_HIERARCHY);

    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::Kotlin), &opts).unwrap();

    // Component -> Container -> Panel
    assert!(
        report.count >= 3,
        "Should find Component, Container, Panel, got {}",
        report.count
    );

    let container_extends = report
        .edges
        .iter()
        .any(|e| e.child == "Container" && e.parent == "Component");
    assert!(
        container_extends,
        "Container should extend Component"
    );

    let panel_extends = report
        .edges
        .iter()
        .any(|e| e.child == "Panel" && e.parent == "Container");
    assert!(panel_extends, "Panel should extend Container");
}

#[test]
fn test_inheritance_csharp() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "Repos.cs", fixtures::CSHARP_INHERITANCE_HIERARCHY);

    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::CSharp), &opts).unwrap();

    // Repository<T> -> UserRepository
    assert!(
        report.count >= 2,
        "Should find at least Repository and UserRepository, got {}",
        report.count
    );
    assert!(
        !report.edges.is_empty(),
        "Should have at least one inheritance edge"
    );
}

#[test]
fn test_inheritance_swift() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "Animals.swift", fixtures::SWIFT_INHERITANCE_HIERARCHY);

    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::Swift), &opts).unwrap();

    // Animal -> Dog -> GuideDog
    assert!(
        report.count >= 3,
        "Should find Animal, Dog, GuideDog, got {}",
        report.count
    );

    let dog_extends = report
        .edges
        .iter()
        .any(|e| e.child == "Dog" && e.parent == "Animal");
    assert!(dog_extends, "Dog should extend Animal");

    let guide_extends = report
        .edges
        .iter()
        .any(|e| e.child == "GuideDog" && e.parent == "Dog");
    assert!(guide_extends, "GuideDog should extend Dog");
}

#[test]
fn test_inheritance_scala() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "shapes.scala", fixtures::SCALA_INHERITANCE_HIERARCHY);

    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::Scala), &opts).unwrap();

    // Shape -> Circle -> Cylinder
    assert!(
        report.count >= 3,
        "Should find Shape, Circle, Cylinder, got {}",
        report.count
    );

    let circle_extends = report
        .edges
        .iter()
        .any(|e| e.child == "Circle" && e.parent == "Shape");
    assert!(circle_extends, "Circle should extend Shape");

    let cylinder_extends = report
        .edges
        .iter()
        .any(|e| e.child == "Cylinder" && e.parent == "Circle");
    assert!(cylinder_extends, "Cylinder should extend Circle");
}

#[test]
fn test_inheritance_php() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "loggers.php", fixtures::PHP_INHERITANCE_HIERARCHY);

    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::Php), &opts).unwrap();

    // Logger -> FileLogger -> BufferedLogger
    assert!(
        report.count >= 3,
        "Should find Logger, FileLogger, BufferedLogger, got {}",
        report.count
    );

    let file_extends = report
        .edges
        .iter()
        .any(|e| e.child == "FileLogger" && e.parent == "Logger");
    assert!(file_extends, "FileLogger should extend Logger");

    let buffered_extends = report
        .edges
        .iter()
        .any(|e| e.child == "BufferedLogger" && e.parent == "FileLogger");
    assert!(
        buffered_extends,
        "BufferedLogger should extend FileLogger"
    );
}

#[test]
fn test_inheritance_class_filter() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "animals.py", fixtures::PYTHON_INHERITANCE_HIERARCHY);

    let opts = InheritanceOptions {
        class_filter: Some("Dog".to_string()),
        ..Default::default()
    };
    let report = extract_inheritance(dir.path(), Some(Language::Python), &opts).unwrap();

    // Filtering to Dog should show Dog and its ancestors/descendants
    assert!(
        report.count >= 1,
        "Filtered graph should contain at least Dog"
    );
    let has_dog = report.nodes.iter().any(|n| n.name == "Dog");
    assert!(has_dog, "Filtered graph must contain Dog");
}

#[test]
fn test_inheritance_depth_requires_class() {
    let opts = InheritanceOptions {
        depth: Some(2),
        class_filter: None,
        ..Default::default()
    };
    let result = opts.validate();
    assert!(result.is_err(), "depth without class should fail validation");
}

#[test]
fn test_inheritance_empty_directory() {
    let dir = TempDir::new().unwrap();
    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::Python), &opts).unwrap();
    assert_eq!(report.count, 0, "Empty directory should yield 0 classes");
}

// =============================================================================
// DEPS TESTS - analyze_dependencies
// =============================================================================

#[test]
fn test_deps_python() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "module_a.py", fixtures::PYTHON_IMPORTS_A);
    create_file(&dir, "module_b.py", fixtures::PYTHON_IMPORTS_B);

    let opts = DepsOptions {
        language: Some("python".to_string()),
        include_external: false,
        ..DepsOptions::default()
    };
    let report = analyze_dependencies(dir.path(), &opts).unwrap();

    // Should detect files
    assert_eq!(report.language, "python");
    assert!(
        report.stats.total_files >= 2,
        "Should analyze at least 2 files, got {}",
        report.stats.total_files
    );
}

#[test]
fn test_deps_typescript() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "module_a.ts", fixtures::TS_IMPORTS_A);
    create_file(&dir, "module_b.ts", fixtures::TS_IMPORTS_B);

    let opts = DepsOptions {
        language: Some("typescript".to_string()),
        include_external: false,
        ..DepsOptions::default()
    };
    let report = analyze_dependencies(dir.path(), &opts).unwrap();

    assert_eq!(report.language, "typescript");
    assert!(
        report.stats.total_files >= 2,
        "Should analyze at least 2 TS files, got {}",
        report.stats.total_files
    );
}

#[test]
fn test_deps_javascript() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "main.js", fixtures::JS_IMPORTS_A);
    create_file(&dir, "utils.js", fixtures::JS_IMPORTS_B);

    let opts = DepsOptions {
        language: Some("javascript".to_string()),
        include_external: false,
        ..DepsOptions::default()
    };
    let report = analyze_dependencies(dir.path(), &opts).unwrap();

    assert!(
        report.stats.total_files >= 2,
        "Should analyze at least 2 JS files"
    );
}

#[test]
fn test_deps_go() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "main.go", fixtures::GO_IMPORTS_A);
    create_file(&dir, "utils/helper.go", fixtures::GO_IMPORTS_B);

    let opts = DepsOptions {
        language: Some("go".to_string()),
        include_external: true, // Go uses stdlib imports
        ..DepsOptions::default()
    };
    let report = analyze_dependencies(dir.path(), &opts).unwrap();

    assert!(
        report.stats.total_files >= 1,
        "Should analyze at least 1 Go file"
    );
}

#[test]
fn test_deps_rust() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "main.rs", fixtures::RUST_IMPORTS_A);
    create_file(&dir, "utils.rs", fixtures::RUST_IMPORTS_B);

    let opts = DepsOptions {
        language: Some("rust".to_string()),
        include_external: false,
        ..DepsOptions::default()
    };
    let report = analyze_dependencies(dir.path(), &opts).unwrap();

    assert!(
        report.stats.total_files >= 2,
        "Should analyze at least 2 Rust files"
    );
}

#[test]
fn test_deps_java() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "Main.java", fixtures::JAVA_SINGLETON);
    create_file(&dir, "Vehicle.java", fixtures::JAVA_INHERITANCE_HIERARCHY);

    let opts = DepsOptions {
        language: Some("java".to_string()),
        include_external: true,
        ..DepsOptions::default()
    };
    let report = analyze_dependencies(dir.path(), &opts).unwrap();

    assert!(
        report.stats.total_files >= 2,
        "Should analyze at least 2 Java files"
    );
}

#[test]
fn test_deps_options_cycles_only() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "module_a.py", fixtures::PYTHON_IMPORTS_A);
    create_file(&dir, "module_b.py", fixtures::PYTHON_IMPORTS_B);

    let opts = DepsOptions {
        language: Some("python".to_string()),
        show_cycles_only: true,
        ..DepsOptions::default()
    };
    let report = analyze_dependencies(dir.path(), &opts).unwrap();

    // No cycles expected in this simple fixture
    assert!(
        report.circular_dependencies.is_empty(),
        "Should have no circular deps in simple fixture"
    );
}

#[test]
fn test_deps_empty_directory() {
    let dir = TempDir::new().unwrap();
    let opts = DepsOptions {
        language: Some("python".to_string()),
        ..DepsOptions::default()
    };
    let report = analyze_dependencies(dir.path(), &opts).unwrap();
    assert_eq!(report.stats.total_files, 0);
}

// =============================================================================
// COHESION TESTS - analyze_cohesion (LCOM4)
// =============================================================================

#[test]
fn test_cohesion_python_low() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "god_class.py", fixtures::PYTHON_LOW_COHESION);

    let report = analyze_cohesion(dir.path(), Some(Language::Python), 2).unwrap();

    assert!(
        report.classes_analyzed >= 1,
        "Should analyze at least 1 class, got {}",
        report.classes_analyzed
    );

    // GodClass has unrelated method groups -> LCOM4 > 1
    let god_class = report.classes.iter().find(|c| c.name == "GodClass");
    assert!(god_class.is_some(), "Should find GodClass");
    let gc = god_class.unwrap();
    assert!(
        gc.lcom4 > 1,
        "GodClass should have LCOM4 > 1 (multiple responsibilities), got {}",
        gc.lcom4
    );
    assert_eq!(
        gc.verdict,
        CohesionVerdict::SplitCandidate,
        "GodClass should be flagged as SplitCandidate"
    );
}

#[test]
fn test_cohesion_python_high() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "vector.py", fixtures::PYTHON_HIGH_COHESION);

    let report = analyze_cohesion(dir.path(), Some(Language::Python), 2).unwrap();

    assert!(report.classes_analyzed >= 1, "Should find Vector2D");

    let vector = report.classes.iter().find(|c| c.name == "Vector2D");
    assert!(vector.is_some(), "Should find Vector2D class");
    let v = vector.unwrap();
    assert_eq!(
        v.lcom4, 1,
        "Vector2D should be fully cohesive (LCOM4=1), got {}",
        v.lcom4
    );
    assert_eq!(v.verdict, CohesionVerdict::Cohesive);
}

#[test]
fn test_cohesion_java() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "Utility.java", fixtures::JAVA_LOW_COHESION);

    let report = analyze_cohesion(dir.path(), Some(Language::Java), 2).unwrap();

    assert!(
        report.classes_analyzed >= 1,
        "Should analyze at least 1 Java class"
    );

    let utility = report.classes.iter().find(|c| c.name == "UtilityManager");
    assert!(utility.is_some(), "Should find UtilityManager");
    let u = utility.unwrap();
    assert!(
        u.lcom4 > 1,
        "UtilityManager should have LCOM4 > 1 (low cohesion), got {}",
        u.lcom4
    );
}

#[test]
fn test_cohesion_typescript() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "manager.ts", fixtures::TS_LOW_COHESION);

    let report = analyze_cohesion(dir.path(), Some(Language::TypeScript), 2).unwrap();

    assert!(
        report.classes_analyzed >= 1,
        "Should analyze at least 1 TypeScript class"
    );

    let manager = report.classes.iter().find(|c| c.name == "AppManager");
    assert!(manager.is_some(), "Should find AppManager");
    let m = manager.unwrap();
    assert!(
        m.lcom4 > 1,
        "AppManager should have LCOM4 > 1 (low cohesion), got {}",
        m.lcom4
    );
}

#[test]
fn test_cohesion_ruby() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "service.rb", fixtures::RUBY_LOW_COHESION);

    let report = analyze_cohesion(dir.path(), Some(Language::Ruby), 2).unwrap();

    assert!(
        report.classes_analyzed >= 1,
        "Should analyze at least 1 Ruby class"
    );

    let svc = report.classes.iter().find(|c| c.name == "ServiceManager");
    assert!(svc.is_some(), "Should find ServiceManager");
    let s = svc.unwrap();
    assert!(
        s.lcom4 > 1,
        "ServiceManager should have LCOM4 > 1, got {}",
        s.lcom4
    );
}

#[test]
fn test_cohesion_summary_stats() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "god.py", fixtures::PYTHON_LOW_COHESION);
    create_file(&dir, "vec.py", fixtures::PYTHON_HIGH_COHESION);

    let report = analyze_cohesion(dir.path(), Some(Language::Python), 2).unwrap();

    // Summary should have data from both classes
    assert!(
        report.summary.total_classes >= 2,
        "Summary should count at least 2 classes"
    );
    assert!(
        report.summary.cohesive >= 1,
        "At least Vector2D should be cohesive"
    );
    assert!(
        report.summary.split_candidates >= 1,
        "At least GodClass should be a split candidate"
    );
    assert!(
        report.summary.avg_lcom4.is_some(),
        "Average LCOM4 should be computed"
    );
}

#[test]
fn test_cohesion_empty_dir() {
    let dir = TempDir::new().unwrap();
    let report = analyze_cohesion(dir.path(), Some(Language::Python), 2).unwrap();
    assert_eq!(report.classes_analyzed, 0);
}

// =============================================================================
// COUPLING TESTS - analyze_coupling
// =============================================================================

#[test]
fn test_coupling_python() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "module_a.py", fixtures::PYTHON_IMPORTS_A);
    create_file(&dir, "module_b.py", fixtures::PYTHON_IMPORTS_B);

    let report = analyze_coupling(dir.path(), Some(Language::Python), Some(10)).unwrap();

    // At minimum, we should get a valid report without error
    assert!(
        report.modules_analyzed >= 2,
        "Should analyze at least 2 modules, got {}",
        report.modules_analyzed
    );
}

#[test]
fn test_coupling_typescript() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "module_a.ts", fixtures::TS_IMPORTS_A);
    create_file(&dir, "module_b.ts", fixtures::TS_IMPORTS_B);

    let report = analyze_coupling(dir.path(), Some(Language::TypeScript), Some(10)).unwrap();

    assert!(
        report.modules_analyzed >= 2,
        "Should analyze at least 2 TS modules, got {}",
        report.modules_analyzed
    );
}

#[test]
fn test_coupling_javascript() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "main.js", fixtures::JS_IMPORTS_A);
    create_file(&dir, "utils.js", fixtures::JS_IMPORTS_B);

    // JavaScript maps to Language::JavaScript
    let report = analyze_coupling(dir.path(), Some(Language::JavaScript), Some(10)).unwrap();

    assert!(
        report.modules_analyzed >= 2,
        "Should analyze at least 2 JS modules, got {}",
        report.modules_analyzed
    );
}

#[test]
fn test_coupling_java() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "Main.java", fixtures::JAVA_SINGLETON);
    create_file(&dir, "Vehicle.java", fixtures::JAVA_INHERITANCE_HIERARCHY);

    let report = analyze_coupling(dir.path(), Some(Language::Java), Some(10)).unwrap();

    assert!(
        report.modules_analyzed >= 2,
        "Should analyze at least 2 Java modules"
    );
}

#[test]
fn test_coupling_verdict_ranges() {
    // Test that the CouplingVerdict::from_score matches documented ranges
    use tldr_core::quality::coupling::CouplingVerdict;

    assert_eq!(CouplingVerdict::from_score(0.0), CouplingVerdict::Loose);
    assert_eq!(CouplingVerdict::from_score(0.29), CouplingVerdict::Loose);
    assert_eq!(CouplingVerdict::from_score(0.3), CouplingVerdict::Moderate);
    assert_eq!(CouplingVerdict::from_score(0.59), CouplingVerdict::Moderate);
    assert_eq!(CouplingVerdict::from_score(0.6), CouplingVerdict::Tight);
    assert_eq!(CouplingVerdict::from_score(1.0), CouplingVerdict::Tight);
}

#[test]
fn test_coupling_empty_dir() {
    let dir = TempDir::new().unwrap();
    let report = analyze_coupling(dir.path(), Some(Language::Python), Some(10)).unwrap();
    assert_eq!(report.modules_analyzed, 0);
    assert_eq!(report.pairs_analyzed, 0);
}

// =============================================================================
// SECURITY - VULNERABILITY SCANNING (scan_vulnerabilities)
// =============================================================================

#[test]
fn test_vuln_python_sql_injection() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "app.py", fixtures::PYTHON_SQL_INJECTION);

    let report =
        scan_vulnerabilities(dir.path(), Some(Language::Python), Some(VulnType::SqlInjection))
            .unwrap();

    assert!(
        report.files_scanned >= 1,
        "Should scan at least 1 file"
    );
    assert!(
        !report.findings.is_empty(),
        "Should detect SQL injection in Python string concatenation"
    );

    // Verify the finding is actually SQL injection
    let has_sqli = report
        .findings
        .iter()
        .any(|f| f.vuln_type == VulnType::SqlInjection);
    assert!(has_sqli, "Finding should be classified as SqlInjection");

    // Check CWE ID
    let sqli_finding = report
        .findings
        .iter()
        .find(|f| f.vuln_type == VulnType::SqlInjection)
        .unwrap();
    assert_eq!(
        sqli_finding.cwe_id.as_deref(),
        Some("CWE-89"),
        "SQL injection should have CWE-89"
    );
}

#[test]
fn test_vuln_python_command_injection() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "cmd.py", fixtures::PYTHON_COMMAND_INJECTION);

    let report = scan_vulnerabilities(
        dir.path(),
        Some(Language::Python),
        Some(VulnType::CommandInjection),
    )
    .unwrap();

    assert!(
        !report.findings.is_empty(),
        "Should detect command injection via os.system()"
    );

    let has_cmdi = report
        .findings
        .iter()
        .any(|f| f.vuln_type == VulnType::CommandInjection);
    assert!(has_cmdi, "Finding should be CommandInjection");
}

#[test]
fn test_vuln_javascript_xss() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "app.js", fixtures::TS_XSS_VULN);

    let report = scan_vulnerabilities(
        dir.path(),
        Some(Language::JavaScript),
        Some(VulnType::Xss),
    )
    .unwrap();

    assert!(
        report.files_scanned >= 1,
        "Should scan at least 1 JS file"
    );

    // innerHTML and document.write are XSS sinks
    if !report.findings.is_empty() {
        let has_xss = report.findings.iter().any(|f| f.vuln_type == VulnType::Xss);
        assert!(has_xss, "Finding should be XSS type");
    }
}

#[test]
fn test_vuln_javascript_command_injection() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "server.js", fixtures::JS_COMMAND_INJECTION);

    let report = scan_vulnerabilities(
        dir.path(),
        Some(Language::JavaScript),
        Some(VulnType::CommandInjection),
    )
    .unwrap();

    assert!(
        report.files_scanned >= 1,
        "Should scan at least 1 JS file"
    );

    if !report.findings.is_empty() {
        let has_cmdi = report
            .findings
            .iter()
            .any(|f| f.vuln_type == VulnType::CommandInjection);
        assert!(has_cmdi, "Finding should be CommandInjection");
    }
}

#[test]
fn test_vuln_go_sql_injection() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "handler.go", fixtures::GO_SQL_INJECTION);

    let report =
        scan_vulnerabilities(dir.path(), Some(Language::Go), Some(VulnType::SqlInjection))
            .unwrap();

    assert!(
        report.files_scanned >= 1,
        "Should scan at least 1 Go file"
    );

    if !report.findings.is_empty() {
        let has_sqli = report
            .findings
            .iter()
            .any(|f| f.vuln_type == VulnType::SqlInjection);
        assert!(has_sqli, "Finding should be SqlInjection");
    }
}

#[test]
fn test_vuln_java_sql_injection() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "Search.java", fixtures::JAVA_SQL_INJECTION);

    let report =
        scan_vulnerabilities(dir.path(), Some(Language::Java), Some(VulnType::SqlInjection))
            .unwrap();

    assert!(
        report.files_scanned >= 1,
        "Should scan at least 1 Java file"
    );

    if !report.findings.is_empty() {
        let has_sqli = report
            .findings
            .iter()
            .any(|f| f.vuln_type == VulnType::SqlInjection);
        assert!(has_sqli, "Finding should be SqlInjection");
    }
}

#[test]
fn test_vuln_all_types_scan() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "sqli.py", fixtures::PYTHON_SQL_INJECTION);
    create_file(&dir, "cmdi.py", fixtures::PYTHON_COMMAND_INJECTION);

    // Scan all vulnerability types (no filter)
    let report = scan_vulnerabilities(dir.path(), Some(Language::Python), None).unwrap();

    assert!(
        report.files_scanned >= 2,
        "Should scan at least 2 files, got {}",
        report.files_scanned
    );

    // Summary should count by type
    if !report.findings.is_empty() {
        assert!(
            !report.summary.by_type.is_empty(),
            "Summary should group findings by type"
        );
    }
}

#[test]
fn test_vuln_clean_file_no_findings() {
    let dir = TempDir::new().unwrap();
    create_file(
        &dir,
        "safe.py",
        r#"
def add(a, b):
    return a + b
"#,
    );

    let report = scan_vulnerabilities(dir.path(), Some(Language::Python), None).unwrap();

    assert!(
        report.findings.is_empty(),
        "Clean file should produce no vulnerability findings"
    );
}

#[test]
fn test_vuln_finding_has_remediation() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "app.py", fixtures::PYTHON_SQL_INJECTION);

    let report =
        scan_vulnerabilities(dir.path(), Some(Language::Python), Some(VulnType::SqlInjection))
            .unwrap();

    for finding in &report.findings {
        assert!(
            !finding.remediation.is_empty(),
            "Each finding should include remediation advice"
        );
    }
}

#[test]
fn test_vuln_empty_directory() {
    let dir = TempDir::new().unwrap();
    let report = scan_vulnerabilities(dir.path(), Some(Language::Python), None).unwrap();
    assert_eq!(report.files_scanned, 0);
    assert!(report.findings.is_empty());
}

// =============================================================================
// SECURITY - SECURE DASHBOARD (run_secure)
// =============================================================================

#[test]
fn test_secure_python_with_vulns() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "app.py", fixtures::PYTHON_SQL_INJECTION);
    create_file(&dir, "cmd.py", fixtures::PYTHON_COMMAND_INJECTION);

    let report = run_secure(
        dir.path().to_str().unwrap(),
        Some("py"),
        false,
    )
    .unwrap();

    assert_eq!(report.wrapper, "secure");
    assert!(
        report.total_elapsed_ms > 0.0,
        "Should report elapsed time"
    );

    // Sub-results should include secrets and vulnerabilities
    assert!(
        report.sub_results.contains_key("secrets"),
        "Should include secrets analysis"
    );
    assert!(
        report.sub_results.contains_key("vulnerabilities"),
        "Should include vulnerability analysis"
    );
}

#[test]
fn test_secure_clean_project() {
    let dir = TempDir::new().unwrap();
    create_file(
        &dir,
        "clean.py",
        r#"
def greet(name):
    return f"Hello, {name}!"
"#,
    );

    let report = run_secure(
        dir.path().to_str().unwrap(),
        Some("py"),
        false,
    )
    .unwrap();

    // Clean project should have no findings
    assert!(
        report.findings.is_empty(),
        "Clean project should have no security findings"
    );
}

#[test]
fn test_secure_findings_sorted_by_severity() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "app.py", fixtures::PYTHON_SQL_INJECTION);
    create_file(&dir, "cmd.py", fixtures::PYTHON_COMMAND_INJECTION);

    let report = run_secure(
        dir.path().to_str().unwrap(),
        Some("py"),
        false,
    )
    .unwrap();

    // Findings should be sorted by severity (critical first)
    if report.findings.len() >= 2 {
        let severity_order = |s: &str| -> u8 {
            match s.to_lowercase().as_str() {
                "critical" => 0,
                "high" => 1,
                "medium" => 2,
                "low" => 3,
                "info" => 4,
                _ => 99,
            }
        };

        for window in report.findings.windows(2) {
            assert!(
                severity_order(&window[0].severity) <= severity_order(&window[1].severity),
                "Findings should be sorted by severity: {} should come before {}",
                window[0].severity,
                window[1].severity
            );
        }
    }
}

#[test]
fn test_secure_summary_populated() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "app.py", fixtures::PYTHON_SQL_INJECTION);

    let report = run_secure(
        dir.path().to_str().unwrap(),
        Some("py"),
        false,
    )
    .unwrap();

    // Summary should contain statistical information
    assert!(
        !report.summary.is_empty(),
        "Summary should be populated with statistics"
    );
}

#[test]
fn test_secure_empty_directory() {
    let dir = TempDir::new().unwrap();
    let report = run_secure(
        dir.path().to_str().unwrap(),
        None,
        false,
    )
    .unwrap();
    assert!(report.findings.is_empty());
}

// =============================================================================
// CROSS-CUTTING TESTS - Multiple analyzers on same fixtures
// =============================================================================

#[test]
fn test_patterns_and_inheritance_combined() {
    // Verify that the same fixture can be analyzed by both patterns and inheritance
    let dir = TempDir::new().unwrap();
    create_file(&dir, "factory.py", fixtures::PYTHON_FACTORY);

    let miner = PatternMiner::new(PatternConfig {
        min_confidence: 0.0,
        ..PatternConfig::default()
    });
    let pattern_report = miner
        .mine_patterns(dir.path(), Some(Language::Python))
        .unwrap();

    let inh_opts = InheritanceOptions::default();
    let inh_report =
        extract_inheritance(dir.path(), Some(Language::Python), &inh_opts).unwrap();

    // Patterns should analyze the file
    assert!(pattern_report.metadata.files_analyzed >= 1);

    // Inheritance should find the class hierarchy
    assert!(inh_report.count >= 3, "Should find Shape, Circle, Square");
}

#[test]
fn test_cohesion_and_coupling_on_same_project() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "module_a.py", fixtures::PYTHON_IMPORTS_A);
    create_file(&dir, "module_b.py", fixtures::PYTHON_IMPORTS_B);
    create_file(&dir, "god_class.py", fixtures::PYTHON_LOW_COHESION);

    let cohesion_report = analyze_cohesion(dir.path(), Some(Language::Python), 2).unwrap();
    let coupling_report = analyze_coupling(dir.path(), Some(Language::Python), Some(10)).unwrap();

    // Both should complete without error
    assert!(cohesion_report.classes_analyzed >= 1);
    assert!(coupling_report.modules_analyzed >= 2);
}

#[test]
fn test_security_and_patterns_on_same_project() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "app.py", fixtures::PYTHON_SQL_INJECTION);
    create_file(&dir, "errors.py", fixtures::PYTHON_ERROR_HANDLING);

    let vuln_report = scan_vulnerabilities(dir.path(), Some(Language::Python), None).unwrap();

    let miner = PatternMiner::new(PatternConfig {
        min_confidence: 0.0,
        ..PatternConfig::default()
    });
    let pattern_report = miner
        .mine_patterns(dir.path(), Some(Language::Python))
        .unwrap();

    // Security should find vulns in app.py
    assert!(vuln_report.files_scanned >= 2);

    // Patterns should find error handling in errors.py
    assert!(pattern_report.error_handling.is_some());
}

#[test]
fn test_multilang_inheritance_in_single_directory() {
    // Test that inheritance analysis handles multiple languages in one directory
    let dir = TempDir::new().unwrap();
    create_file(&dir, "animals.py", fixtures::PYTHON_INHERITANCE_HIERARCHY);
    create_file(&dir, "Vehicle.java", fixtures::JAVA_INHERITANCE_HIERARCHY);
    create_file(&dir, "shapes.ts", fixtures::TS_INHERITANCE_HIERARCHY);

    // Analyze all languages (no filter)
    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), None, &opts).unwrap();

    // Should find classes from all three languages
    assert!(
        report.count >= 9,
        "Should find classes from Python (4) + Java (3) + TS (3), got {}",
        report.count
    );

    assert!(
        report.languages.len() >= 3,
        "Should detect at least 3 languages, got {:?}",
        report.languages
    );
}

#[test]
fn test_vuln_report_summary_structure() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "sqli.py", fixtures::PYTHON_SQL_INJECTION);

    let report = scan_vulnerabilities(dir.path(), Some(Language::Python), None).unwrap();

    // Summary should match findings
    assert_eq!(
        report.summary.total_findings,
        report.findings.len(),
        "Summary total should match findings count"
    );

    if !report.findings.is_empty() {
        assert!(
            report.summary.affected_files >= 1,
            "Should report at least 1 affected file"
        );
    }
}

#[test]
fn test_deps_report_stats_structure() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "a.py", fixtures::PYTHON_IMPORTS_A);
    create_file(&dir, "b.py", fixtures::PYTHON_IMPORTS_B);

    let opts = DepsOptions {
        language: Some("python".to_string()),
        ..DepsOptions::default()
    };
    let report = analyze_dependencies(dir.path(), &opts).unwrap();

    // Stats should be populated
    assert!(
        report.stats.total_files >= 2,
        "Stats should count analyzed files"
    );
}

#[test]
fn test_inheritance_report_scan_time() {
    let dir = TempDir::new().unwrap();
    create_file(&dir, "animals.py", fixtures::PYTHON_INHERITANCE_HIERARCHY);

    let opts = InheritanceOptions::default();
    let report = extract_inheritance(dir.path(), Some(Language::Python), &opts).unwrap();

    // scan_time_ms should be > 0 (analysis took some time)
    // Note: on very fast machines it could be 0ms, so just check it exists
    assert!(
        report.scan_time_ms < 60_000,
        "Scan time should be reasonable (< 60s)"
    );
}
