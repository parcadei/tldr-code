//! Purity Command - Effect/Purity Analysis
//!
//! Analyzes function purity (side-effect free) across a file or directory.
//!
//! # Algorithm
//!
//! 1. For each function, detect direct effects:
//!    - I/O operations (open, print, read, write, input)
//!    - Global variable writes
//!    - Attribute writes (self.x = ...)
//!    - Collection mutations
//! 2. If interprocedural enabled:
//!    - Build call graph
//!    - Propagate impurity from callees to callers
//! 3. Assign confidence based on analysis completeness
//!
//! # Example
//!
//! ```bash
//! # Analyze a single file
//! tldr purity src/utils.py
//!
//! # Analyze a directory
//! tldr purity src/ --include-tests
//!
//! # Disable interprocedural propagation
//! tldr purity src/utils.py --no-interprocedural
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use clap::Args;
use tree_sitter::{Node, Parser};

use super::error::{PatternsError, PatternsResult};
use super::types::{Confidence, FilePurityReport, FunctionPurity, OutputFormat, PurityReport};
use super::validation::{
    check_directory_file_count, read_file_safe, validate_directory_path, validate_file_path,
    validate_file_path_in_project, MAX_DIRECTORY_FILES,
};
use tldr_core::ast::parser::ParserPool;
use tldr_core::types::Language;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Analyze function purity (side-effect free) across a file or directory.
#[derive(Debug, Args)]
pub struct PurityArgs {
    /// File or directory to analyze
    pub path: PathBuf,

    /// Specific function to analyze
    #[arg(long)]
    pub function: Option<String>,

    /// Disable interprocedural propagation
    #[arg(long)]
    pub no_interprocedural: bool,

    /// Include test files in directory analysis
    #[arg(long)]
    pub include_tests: bool,

    /// Output format (json or text). Prefer global --format/-f flag.
    #[arg(long = "output", short = 'o', hide = true, default_value = "json", value_enum)]
    pub output_format: OutputFormat,

    /// Project root for path validation (optional)
    #[arg(long)]
    pub project_root: Option<PathBuf>,
}

impl PurityArgs {
    /// Run the purity analysis command
    pub fn run(&self) -> anyhow::Result<()> {
        run(self.clone())
    }
}

impl Clone for PurityArgs {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            function: self.function.clone(),
            no_interprocedural: self.no_interprocedural,
            include_tests: self.include_tests,
            output_format: self.output_format,
            project_root: self.project_root.clone(),
        }
    }
}

// =============================================================================
// Constants - I/O Operations and Impure Calls
// =============================================================================

/// Known I/O operations that make a function impure
pub const IO_OPERATIONS: &[&str] = &[
    // stdout
    "print",
    // File I/O
    "open",
    "read",
    "write",
    "readline",
    "readlines",
    "writelines",
    "input",
    // System
    "system",
    "popen",
    "exec",
    "eval",
    // Network
    "request",
    "fetch",
    "urlopen",
    // Database
    "execute",
    "executemany",
    "fetchone",
    "fetchall",
];

/// Known impure calls (non-deterministic or side-effecting)
pub const IMPURE_CALLS: &[&str] = &[
    // Random
    "random",
    "randint",
    "choice",
    "shuffle",
    "sample",
    "uniform",
    "random.random",
    "random.randint",
    "random.choice",
    "random.shuffle",
    "random.sample",
    "random.uniform",
    // Time
    "time",
    "time.time",
    "datetime.now",
    "datetime.datetime.now",
    // UUID
    "uuid4",
    "uuid1",
    "uuid.uuid4",
    "uuid.uuid1",
    // Logging
    "logging.info",
    "logging.debug",
    "logging.warning",
    "logging.error",
    "logging.critical",
    "logger.info",
    "logger.debug",
    "logger.warning",
    "logger.error",
    // Subprocess
    "subprocess.run",
    "subprocess.call",
    "subprocess.Popen",
    "subprocess.check_output",
    // OS
    "os.system",
    "os.popen",
    "os.getenv",
    "os.environ",
    "os.mkdir",
    "os.makedirs",
    "os.remove",
    "os.rename",
    // Network
    "requests.get",
    "requests.post",
    "requests.put",
    "requests.delete",
    "urllib.request.urlopen",
];

/// Collection mutation methods
const COLLECTION_MUTATIONS: &[&str] = &[
    "append", "extend", "insert", "remove", "pop", "clear", "update", "add", "discard",
    "setdefault", "sort", "reverse",
];

// =============================================================================
// Per-Language Side-Effect Detection Constants
// =============================================================================

// JavaScript/TypeScript side effects
const JS_IO_OPERATIONS: &[&str] = &[
    "console.log", "console.error", "console.warn",
    "fetch",
];
const JS_IMPURE_CALLS: &[&str] = &[
    "Math.random", "Date.now",
    "setTimeout", "setInterval",
];
const JS_COLLECTION_MUTATIONS: &[&str] = &[
    "push", "pop", "splice", "sort",
];

// Rust side effects  
const RUST_IO_OPERATIONS: &[&str] = &[
    "println!", "eprintln!", "print!",
];
const RUST_IMPURE_CALLS: &[&str] = &[
    "std::process::exit", "panic!",
];

// Go side effects
const GO_IO_OPERATIONS: &[&str] = &[
    "fmt.Println", "fmt.Printf",
];
const GO_IMPURE_CALLS: &[&str] = &[
    "rand.Int", "time.Now",
];

// Java side effects
const JAVA_IO_OPERATIONS: &[&str] = &[
    "System.out.println", "System.err.println",
];
const JAVA_IMPURE_CALLS: &[&str] = &[
    "Math.random", "Thread.sleep",
];

// C/C++ side effects
const C_IO_OPERATIONS: &[&str] = &[
    "printf", "fprintf", "scanf",
];
const C_IMPURE_CALLS: &[&str] = &[
    "rand", "srand", "time",
];

// Ruby side effects
const RUBY_IO_OPERATIONS: &[&str] = &[
    "puts", "print", "p",
];
const RUBY_IMPURE_CALLS: &[&str] = &[
    "rand", "sleep", "exit",
];

// PHP side effects
const PHP_IO_OPERATIONS: &[&str] = &[
    "echo", "print", "file_get_contents",
];
const PHP_IMPURE_CALLS: &[&str] = &[
    "rand", "time", "date",
];

// Kotlin side effects
const KOTLIN_IO_OPERATIONS: &[&str] = &[
    "println", "print",
];
const KOTLIN_IMPURE_CALLS: &[&str] = &[
    "Random.nextInt", "System.currentTimeMillis",
];

// Swift side effects
const SWIFT_IO_OPERATIONS: &[&str] = &[
    "print", "readLine",
];
const SWIFT_IMPURE_CALLS: &[&str] = &[
    "arc4random", "Date",
];

// C# side effects
const CSHARP_IO_OPERATIONS: &[&str] = &[
    "Console.WriteLine", "Console.ReadLine",
];
const CSHARP_IMPURE_CALLS: &[&str] = &[
    "Random.Next", "DateTime.Now",
];

// Scala side effects
const SCALA_IO_OPERATIONS: &[&str] = &[
    "println", "print",
];
const SCALA_IMPURE_CALLS: &[&str] = &[
    "Random.nextInt", "Random.nextDouble", "System.currentTimeMillis",
];

// Elixir side effects
const ELIXIR_IO_OPERATIONS: &[&str] = &[
    "IO.puts", "IO.inspect",
];
const ELIXIR_IMPURE_CALLS: &[&str] = &[
    ":rand.uniform", "System.cmd",
];

// Lua side effects
const LUA_IO_OPERATIONS: &[&str] = &[
    "print", "io.open",
];
const LUA_IMPURE_CALLS: &[&str] = &[
    "math.random", "os.time",
];

// OCaml side effects
const OCAML_IO_OPERATIONS: &[&str] = &[
    "print_string", "print_endline",
];
const OCAML_IMPURE_CALLS: &[&str] = &[
    "Random.int", "Unix.time",
];

// =============================================================================
// Per-Language Dispatch Helpers
// =============================================================================

/// Return the I/O operations array for a given language
fn io_ops_for_language(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => IO_OPERATIONS,
        Language::JavaScript | Language::TypeScript => JS_IO_OPERATIONS,
        Language::Rust => RUST_IO_OPERATIONS,
        Language::Go => GO_IO_OPERATIONS,
        Language::Java => JAVA_IO_OPERATIONS,
        Language::C | Language::Cpp => C_IO_OPERATIONS,
        Language::Ruby => RUBY_IO_OPERATIONS,
        Language::Php => PHP_IO_OPERATIONS,
        Language::Kotlin => KOTLIN_IO_OPERATIONS,
        Language::Swift => SWIFT_IO_OPERATIONS,
        Language::CSharp => CSHARP_IO_OPERATIONS,
        Language::Scala => SCALA_IO_OPERATIONS,
        Language::Elixir => ELIXIR_IO_OPERATIONS,
        Language::Lua | Language::Luau => LUA_IO_OPERATIONS,
        Language::Ocaml => OCAML_IO_OPERATIONS,
    }
}

/// Return the impure calls array for a given language
fn impure_calls_for_language(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => IMPURE_CALLS,
        Language::JavaScript | Language::TypeScript => JS_IMPURE_CALLS,
        Language::Rust => RUST_IMPURE_CALLS,
        Language::Go => GO_IMPURE_CALLS,
        Language::Java => JAVA_IMPURE_CALLS,
        Language::C | Language::Cpp => C_IMPURE_CALLS,
        Language::Ruby => RUBY_IMPURE_CALLS,
        Language::Php => PHP_IMPURE_CALLS,
        Language::Kotlin => KOTLIN_IMPURE_CALLS,
        Language::Swift => SWIFT_IMPURE_CALLS,
        Language::CSharp => CSHARP_IMPURE_CALLS,
        Language::Scala => SCALA_IMPURE_CALLS,
        Language::Elixir => ELIXIR_IMPURE_CALLS,
        Language::Lua | Language::Luau => LUA_IMPURE_CALLS,
        Language::Ocaml => OCAML_IMPURE_CALLS,
    }
}

/// Return the collection mutation methods array for a given language
fn collection_mutations_for_language(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::JavaScript | Language::TypeScript => JS_COLLECTION_MUTATIONS,
        // All other languages fall back to the Python/generic collection mutations
        _ => COLLECTION_MUTATIONS,
    }
}

// =============================================================================
// Effect Analysis Types
// =============================================================================

/// Information about detected effects in a function
#[derive(Debug, Clone, Default)]
struct DetectedEffects {
    /// I/O operations detected
    io_operations: Vec<String>,
    /// Global writes detected
    global_writes: Vec<String>,
    /// Attribute writes detected (self.x = ...)
    attribute_writes: Vec<String>,
    /// Collection mutations detected
    collection_mutations: Vec<String>,
    /// Unknown calls (reduce confidence)
    unknown_calls: Vec<String>,
    /// Known local calls (for interprocedural)
    local_calls: Vec<String>,
    /// Whether any function calls were detected at all (including pure builtins).
    /// Used to distinguish "no calls" from "only known-pure calls".
    has_any_calls: bool,
}

impl DetectedEffects {
    fn is_empty(&self) -> bool {
        self.io_operations.is_empty()
            && self.global_writes.is_empty()
            && self.attribute_writes.is_empty()
            && self.collection_mutations.is_empty()
            && self.unknown_calls.is_empty()
    }

    /// Determine the purity classification based on effects.
    ///
    /// Classification logic:
    /// 1. Has known impure effects (IO, global writes, etc.) -> "impure"
    /// 2. Has unknown/unresolvable calls -> "unknown"
    /// 3. Has calls that all resolved to pure builtins or local functions -> "pure"
    /// 4. Has no calls at all (empty body or pure computation) -> "unknown"
    ///    (absence of evidence is not evidence of purity)
    fn classification(&self) -> &'static str {
        if !self.io_operations.is_empty()
            || !self.global_writes.is_empty()
            || !self.attribute_writes.is_empty()
            || !self.collection_mutations.is_empty()
        {
            "impure"
        } else if !self.unknown_calls.is_empty() {
            "unknown"
        } else if self.has_any_calls || !self.local_calls.is_empty() {
            // All calls resolved to known-pure builtins or local functions.
            // Local function purity is verified by interprocedural propagation.
            "pure"
        } else {
            // No calls detected at all. This could be an empty function body,
            // pure arithmetic (a+b), or a parser failure. Without any calls to
            // evaluate, we have no evidence to claim purity.
            "unknown"
        }
    }

    fn to_effect_strings(&self) -> Vec<String> {
        let mut effects = Vec::new();
        if !self.io_operations.is_empty() {
            effects.push("io".to_string());
        }
        if !self.global_writes.is_empty() {
            effects.push("global_write".to_string());
        }
        if !self.attribute_writes.is_empty() {
            effects.push("attribute_write".to_string());
        }
        if !self.collection_mutations.is_empty() {
            effects.push("collection_modify".to_string());
        }
        effects
    }
}

/// Effect analysis state for interprocedural analysis
#[derive(Debug)]
pub struct EffectAnalysis {
    /// Call graph: function name -> list of called function names
    pub call_graph: HashMap<String, Vec<String>>,
    /// Detected effects per function
    pub effects: HashMap<String, DetectedEffects>,
    /// Purity status per function (true = pure)
    pub purity: HashMap<String, bool>,
}

impl EffectAnalysis {
    fn new() -> Self {
        Self {
            call_graph: HashMap::new(),
            effects: HashMap::new(),
            purity: HashMap::new(),
        }
    }
}

// =============================================================================
// Tree-sitter Multi-Language Parsing
// =============================================================================

/// Initialize tree-sitter parser for Python (legacy, used by Python-only paths)
fn get_python_parser() -> PatternsResult<Parser> {
    get_parser_for_language(Language::Python)
}

/// Initialize tree-sitter parser for a given language
fn get_parser_for_language(lang: Language) -> PatternsResult<Parser> {
    let ts_lang = ParserPool::get_ts_language(lang)
        .ok_or_else(|| PatternsError::parse_error(PathBuf::new(), format!("Unsupported language: {}", lang)))?;
    let mut parser = Parser::new();
    parser
        .set_language(&ts_lang)
        .map_err(|e| PatternsError::parse_error(PathBuf::new(), format!("Failed to set language: {}", e)))?;
    Ok(parser)
}

/// Get function/method node kinds for a language
fn function_kinds_for_language(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => &["function_definition", "async_function_definition"],
        Language::Go => &["function_declaration", "method_declaration"],
        Language::TypeScript | Language::JavaScript => &[
            "function_declaration", "method_definition", "arrow_function",
            "generator_function_declaration",
        ],
        Language::Rust => &["function_item"],
        Language::Java => &["method_declaration", "constructor_declaration"],
        Language::C | Language::Cpp => &["function_definition"],
        Language::Ruby => &["method", "singleton_method"],
        Language::Php => &["function_definition", "method_declaration"],
        Language::Kotlin => &["function_declaration"],
        Language::Swift => &["function_declaration", "init_declaration"],
        Language::CSharp => &["method_declaration", "constructor_declaration"],
        Language::Scala => &["function_definition", "val_definition"],
        Language::Elixir => &["call"],
        Language::Lua | Language::Luau => &["function_declaration", "local_function"],
        Language::Ocaml => &["let_binding", "value_definition"],
    }
}

/// Get text for a node from source
fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Extract function name from a function definition node (multi-language)
fn get_function_name(node: Node, source: &[u8]) -> Option<String> {
    // Try field-based name extraction first (Go, Rust, Java, TS, etc.)
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = node_text(name_node, source).to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    // C/C++: function_definition -> declarator -> function_declarator -> declarator -> identifier
    if let Some(declarator) = node.child_by_field_name("declarator") {
        if let Some(name) = extract_c_declarator_name(declarator, source) {
            return Some(name);
        }
    }
    // Fallback: first identifier-like child (Python uses "identifier", OCaml uses "value_name",
    // Swift uses "simple_identifier")
    for child in node.children(&mut node.walk()) {
        match child.kind() {
            "identifier" | "value_name" | "simple_identifier" => {
                return Some(node_text(child, source).to_string());
            }
            _ => {}
        }
    }
    None
}

/// Recursively extract function name from C/C++ nested declarator chain
fn extract_c_declarator_name(declarator: Node, source: &[u8]) -> Option<String> {
    match declarator.kind() {
        "identifier" | "field_identifier" => {
            let name = node_text(declarator, source).to_string();
            if !name.is_empty() { Some(name) } else { None }
        }
        "function_declarator" | "pointer_declarator" | "reference_declarator" | "parenthesized_declarator" => {
            declarator.child_by_field_name("declarator")
                .and_then(|inner| extract_c_declarator_name(inner, source))
        }
        _ => None,
    }
}

/// Get the line number (1-indexed) for a node
fn get_line_number(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

// =============================================================================
// Effect Detection
// =============================================================================

/// Extract parameter names from a function definition node (multi-language)
fn extract_parameter_names(func_node: Node, source: &[u8], lang: Language) -> HashSet<String> {
    let mut params = HashSet::new();
    
    match lang {
        Language::Python => {
            // Python: parameters node -> identifier children, or typed_parameter
            if let Some(params_node) = func_node.child_by_field_name("parameters") {
                collect_python_params(params_node, source, &mut params);
            }
        }
        Language::Go => {
            // Go: parameter_list -> parameter_declaration -> identifier children
            if let Some(params_node) = func_node.child_by_field_name("parameters") {
                collect_go_params(params_node, source, &mut params);
            }
            // Also check receiver for method declarations
            if let Some(recv_node) = func_node.child_by_field_name("receiver") {
                collect_go_params(recv_node, source, &mut params);
            }
        }
        Language::JavaScript | Language::TypeScript => {
            // JS/TS: formal_parameters -> identifier or pattern children
            if let Some(params_node) = func_node.child_by_field_name("parameters") {
                collect_js_params(params_node, source, &mut params);
            }
        }
        Language::Rust => {
            // Rust: parameters -> parameter -> pattern -> identifier
            if let Some(params_node) = func_node.child_by_field_name("parameters") {
                collect_rust_params(params_node, source, &mut params);
            }
        }
        Language::Java => {
            // Java: formal_parameters -> formal_parameter -> identifier
            if let Some(params_node) = func_node.child_by_field_name("parameters") {
                collect_java_params(params_node, source, &mut params);
            }
        }
        Language::C | Language::Cpp => {
            // C/C++: parameters -> parameter_declaration -> declarator -> identifier
            if let Some(params_node) = func_node.child_by_field_name("parameters") {
                collect_c_params(params_node, source, &mut params);
            }
        }
        Language::Ruby => {
            // Ruby: method/singleton_method -> parameters -> identifier or optional_parameter etc.
            if let Some(params_node) = func_node.child_by_field_name("parameters") {
                collect_ruby_params(params_node, source, &mut params);
            }
        }
        Language::Php => {
            // PHP: formal_parameters -> simple_parameter -> variable_name -> identifier
            if let Some(params_node) = func_node.child_by_field_name("parameters") {
                collect_php_params(params_node, source, &mut params);
            }
        }
        Language::Kotlin => {
            // Kotlin: function_declaration has "parameters" field with class_parameter children
            if let Some(params_node) = func_node.child_by_field_name("parameters") {
                collect_kotlin_params(params_node, source, &mut params);
            }
        }
        Language::Swift => {
            // Swift: function_declaration -> function_signature -> parameter
            if let Some(sig_node) = func_node.child_by_field_name("signature") {
                collect_swift_params(sig_node, source, &mut params);
            }
        }
        Language::CSharp => {
            // C#: method_declaration -> formal_parameter_list -> parameter
            if let Some(params_node) = func_node.child_by_field_name("parameters") {
                collect_csharp_params(params_node, source, &mut params);
            }
        }
        Language::Scala => {
            // Scala: function_definition -> parameters
            if let Some(params_node) = func_node.child_by_field_name("parameters") {
                collect_scala_params(params_node, source, &mut params);
            }
        }
        Language::Elixir => {
            // Elixir: call (def/defp) -> arguments -> call (function name with params)
            // Already handled in elixir_def_name, we skip params here
        }
        Language::Lua | Language::Luau => {
            // Lua: function_declaration -> parameters -> identifier children
            if let Some(params_node) = func_node.child_by_field_name("parameters") {
                collect_lua_params(params_node, source, &mut params);
            }
        }
        Language::Ocaml => {
            // OCaml: let_binding/value_definition -> parameters
            // OCaml uses curried functions, parameters are nested
            collect_ocaml_params(func_node, source, &mut params);
        }
    }
    
    params
}

/// Collect Python parameter names recursively
fn collect_python_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            params.insert(node_text(node, source).to_string());
        }
        "typed_parameter" | "default_parameter" | "typed_default_parameter" 
        | "keyword_separator" | "list_splat_pattern" | "dictionary_splat_pattern" => {
            // These have identifier as first child
            if let Some(first_child) = node.child(0) {
                if first_child.kind() == "identifier" {
                    params.insert(node_text(first_child, source).to_string());
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_python_params(child, source, params);
            }
        }
    }
}

/// Collect Go parameter names recursively
fn collect_go_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            params.insert(node_text(node, source).to_string());
        }
        "parameter_declaration" => {
            // Go allows multiple names per type: a, b int
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    params.insert(node_text(child, source).to_string());
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_go_params(child, source, params);
            }
        }
    }
}

/// Collect JavaScript/TypeScript parameter names recursively
fn collect_js_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            params.insert(node_text(node, source).to_string());
        }
        "required_parameter" | "optional_parameter" => {
            // Try pattern field first
            if let Some(pattern) = node.child_by_field_name("pattern") {
                collect_js_params(pattern, source, params);
            } else {
                // Fall back to first child
                if let Some(first) = node.child(0) {
                    collect_js_params(first, source, params);
                }
            }
        }
        "array_pattern" | "object_pattern" => {
            // Destructuring patterns - collect all identifiers inside
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_js_params(child, source, params);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_js_params(child, source, params);
            }
        }
    }
}

/// Collect Rust parameter names recursively
fn collect_rust_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            params.insert(node_text(node, source).to_string());
        }
        "parameter" => {
            // Pattern is the first field or child
            if let Some(pattern) = node.child_by_field_name("pattern") {
                collect_rust_params(pattern, source, params);
            } else {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    collect_rust_params(child, source, params);
                    break; // Only first child for pattern
                }
            }
        }
        "self" | "mutable_specifier" => {
            // Skip self and &mut
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_rust_params(child, source, params);
            }
        }
    }
}

/// Collect Java parameter names recursively
fn collect_java_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            params.insert(node_text(node, source).to_string());
        }
        "formal_parameter" | "receiver_parameter" => {
            // receiver_parameter has 'this' which we skip
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    params.insert(node_text(child, source).to_string());
                }
            }
        }
        "spread_parameter" => {
            // varargs: Type... name
            if let Some(name) = node.child_by_field_name("name") {
                collect_java_params(name, source, params);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_java_params(child, source, params);
            }
        }
    }
}

/// Collect C/C++ parameter names recursively
fn collect_c_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            params.insert(node_text(node, source).to_string());
        }
        "parameter_declaration" => {
            // Look for declarator which contains the name
            if let Some(declarator) = node.child_by_field_name("declarator") {
                // Declarator might be nested (pointer, reference, etc.)
                collect_c_params(declarator, source, params);
            }
        }
        "pointer_declarator" | "reference_declarator" | "parenthesized_declarator" |
        "function_declarator" | "abstract_parenthesized_declarator" => {
            // Navigate through declarator chains
            if let Some(declarator) = node.child_by_field_name("declarator") {
                collect_c_params(declarator, source, params);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_c_params(child, source, params);
            }
        }
    }
}

/// Collect Ruby parameter names recursively
fn collect_ruby_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            params.insert(node_text(node, source).to_string());
        }
        "optional_parameter" | "keyword_parameter" | "splat_parameter" | 
        "hash_splat_parameter" | "block_parameter" => {
            if let Some(name) = node.child_by_field_name("name") {
                params.insert(node_text(name, source).to_string());
            } else if let Some(first) = node.child(0) {
                if first.kind() == "identifier" {
                    params.insert(node_text(first, source).to_string());
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_ruby_params(child, source, params);
            }
        }
    }
}

/// Collect PHP parameter names recursively
fn collect_php_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "simple_parameter" => {
            // variable_name -> identifier
            if let Some(var_name) = node.child_by_field_name("name") {
                if var_name.kind() == "variable_name" {
                    if let Some(ident) = var_name.child(0) {
                        if ident.kind() == "identifier" || ident.kind() == "name" {
                            params.insert(node_text(ident, source).to_string());
                        }
                    }
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_php_params(child, source, params);
            }
        }
    }
}

/// Collect Kotlin parameter names recursively
fn collect_kotlin_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "class_parameter" | "parameter" => {
            // parameter has "name" field
            if let Some(name) = node.child_by_field_name("name") {
                params.insert(node_text(name, source).to_string());
            }
        }
        "simple_identifier" => {
            params.insert(node_text(node, source).to_string());
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_kotlin_params(child, source, params);
            }
        }
    }
}

/// Collect Swift parameter names recursively
fn collect_swift_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "parameter" => {
            // parameter has "name" field
            if let Some(name) = node.child_by_field_name("name") {
                params.insert(node_text(name, source).to_string());
            }
        }
        "simple_identifier" => {
            params.insert(node_text(node, source).to_string());
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_swift_params(child, source, params);
            }
        }
    }
}

/// Collect C# parameter names recursively
fn collect_csharp_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "parameter" => {
            // parameter has "name" field
            if let Some(name) = node.child_by_field_name("name") {
                params.insert(node_text(name, source).to_string());
            }
        }
        "identifier" => {
            params.insert(node_text(node, source).to_string());
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_csharp_params(child, source, params);
            }
        }
    }
}

/// Collect Scala parameter names recursively
fn collect_scala_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            params.insert(node_text(node, source).to_string());
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_scala_params(child, source, params);
            }
        }
    }
}

/// Collect Lua parameter names recursively
fn collect_lua_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            params.insert(node_text(node, source).to_string());
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_lua_params(child, source, params);
            }
        }
    }
}

/// Collect OCaml parameter names (curried functions)
fn collect_ocaml_params(node: Node, source: &[u8], params: &mut HashSet<String>) {
    // OCaml uses curried functions: let f x y = ...
    // Parameters are value_name nodes before the function body
    match node.kind() {
        "value_name" => {
            params.insert(node_text(node, source).to_string());
        }
        "let_binding" | "value_definition" => {
            // Look for value_name children before the function body
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "value_name" => {
                        // First value_name is the function name, subsequent are params
                        // We handle this at a higher level
                    }
                    "fun_expression" => {
                        // fun x -> ... syntax
                        collect_ocaml_params(child, source, params);
                    }
                    _ => {
                        collect_ocaml_params(child, source, params);
                    }
                }
            }
        }
        "fun_expression" => {
            // fun x y -> ...
            if let Some(param) = node.child_by_field_name("parameter") {
                collect_ocaml_params(param, source, params);
            }
        }
        "parameter" => {
            if let Some(pat) = node.child_by_field_name("pattern") {
                collect_ocaml_params(pat, source, params);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_ocaml_params(child, source, params);
            }
        }
    }
}

/// Extract local variable names from a function body (multi-language)
fn extract_local_variables(func_node: Node, source: &[u8], lang: Language) -> HashSet<String> {
    let mut locals = HashSet::new();
    
    // Find the body node to search within
    let body_node = match lang {
        Language::Python => func_node.child_by_field_name("body"),
        Language::Go => func_node.child_by_field_name("body"),
        Language::JavaScript | Language::TypeScript => func_node.child_by_field_name("body"),
        Language::Rust => func_node.child_by_field_name("body"),
        Language::Java => func_node.child_by_field_name("body"),
        Language::C | Language::Cpp => func_node.child_by_field_name("body"),
        Language::Ruby => func_node.child_by_field_name("body"),
        Language::Php => func_node.child_by_field_name("body"),
        Language::Kotlin => func_node.child_by_field_name("body"),
        Language::Swift => func_node.child_by_field_name("body"),
        Language::CSharp => func_node.child_by_field_name("body"),
        Language::Scala => func_node.child_by_field_name("body"),
        Language::Elixir => {
            // Find do_block
            let mut cursor = func_node.walk();
            let mut found = None;
            for child in func_node.children(&mut cursor) {
                if child.kind() == "do_block" {
                    found = Some(child);
                    break;
                }
            }
            found
        }
        Language::Lua | Language::Luau => func_node.child_by_field_name("body"),
        Language::Ocaml => func_node.child_by_field_name("body"),
    };
    
    if let Some(body) = body_node {
        collect_local_vars_recursive(body, source, lang, &mut locals);
    }
    
    locals
}

/// Recursively collect local variable declarations
fn collect_local_vars_recursive(node: Node, source: &[u8], lang: Language, locals: &mut HashSet<String>) {
    match lang {
        Language::Python => collect_python_locals(node, source, locals),
        Language::Go => collect_go_locals(node, source, locals),
        Language::JavaScript | Language::TypeScript => collect_js_locals(node, source, locals),
        Language::Rust => collect_rust_locals(node, source, locals),
        Language::Java => collect_java_locals(node, source, locals),
        Language::C | Language::Cpp => collect_c_locals(node, source, locals),
        Language::Ruby => collect_ruby_locals(node, source, locals),
        Language::Php => collect_php_locals(node, source, locals),
        Language::Kotlin => collect_kotlin_locals(node, source, locals),
        Language::Swift => collect_swift_locals(node, source, locals),
        Language::CSharp => collect_csharp_locals(node, source, locals),
        Language::Scala => collect_scala_locals(node, source, locals),
        Language::Elixir => collect_elixir_locals(node, source, locals),
        Language::Lua | Language::Luau => collect_lua_locals(node, source, locals),
        Language::Ocaml => collect_ocaml_locals(node, source, locals),
    }
}

/// Collect Python local variable declarations
fn collect_python_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "assignment" | "augmented_assignment" => {
            // Left side can have targets
            if let Some(left) = node.child_by_field_name("left") {
                collect_assignment_targets(left, source, locals);
            }
        }
        "for_statement" => {
            // for target in iterable:
            if let Some(left) = node.child_by_field_name("left") {
                collect_assignment_targets(left, source, locals);
            }
        }
        "with_statement" => {
            // with ctx as var:
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "with_clause" || child.kind() == "with_item" {
                    // Look for 'as' pattern
                    collect_python_locals(child, source, locals);
                }
            }
        }
        "as_pattern" | "with_item" => {
            if let Some(alias) = node.child_by_field_name("alias") {
                collect_assignment_targets(alias, source, locals);
            }
        }
        "except_clause" => {
            // except Exception as e:
            if let Some(alias) = node.child_by_field_name("alias") {
                collect_assignment_targets(alias, source, locals);
            }
        }
        "list_comprehension" | "set_comprehension" | "generator_expression" => {
            // [x for x in items] - x is local to comprehension
            if let Some(name) = node.child_by_field_name("name") {
                collect_assignment_targets(name, source, locals);
            }
        }
        "dictionary_comprehension" => {
            // {k: v for k, v in items}
            if let Some(name) = node.child_by_field_name("name") {
                collect_assignment_targets(name, source, locals);
            }
        }
        _ => {
            // Recurse into children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_python_locals(child, source, locals);
            }
        }
    }
}

/// Collect assignment targets (handles tuples, lists, etc.)
fn collect_assignment_targets(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            locals.insert(node_text(node, source).to_string());
        }
        "pattern_list" | "tuple_pattern" | "list_pattern" => {
            // (a, b) = ... or [a, b] = ...
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_assignment_targets(child, source, locals);
            }
        }
        _ => {
            // For other nodes, try to find identifier children
            if let Some(ident) = node.child_by_field_name("name") {
                if ident.kind() == "identifier" {
                    locals.insert(node_text(ident, source).to_string());
                }
            }
        }
    }
}

/// Collect Go local variable declarations
fn collect_go_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "short_var_declaration" => {
            // a, b := 1, 2
            if let Some(left) = node.child_by_field_name("left") {
                collect_go_idents(left, source, locals);
            }
        }
        "var_declaration" | "const_declaration" => {
            // var a, b int = 1, 2
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "var_spec" || child.kind() == "const_spec" {
                    if let Some(name) = child.child_by_field_name("name") {
                        locals.insert(node_text(name, source).to_string());
                    }
                }
            }
        }
        "range_clause" => {
            // for k, v := range ...
            if let Some(left) = node.child_by_field_name("left") {
                collect_go_idents(left, source, locals);
            }
        }
        "type_switch_statement" => {
            // switch x := v.(type)
            // Look for type_switch_guard
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_switch_guard" {
                    if let Some(alias) = child.child_by_field_name("alias") {
                        collect_go_idents(alias, source, locals);
                    }
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_go_locals(child, source, locals);
            }
        }
    }
}

/// Collect Go identifiers from a node
fn collect_go_idents(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            locals.insert(node_text(node, source).to_string());
        }
        "expression_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_go_idents(child, source, locals);
            }
        }
        _ => {
            // Recurse for other composite types
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_go_idents(child, source, locals);
            }
        }
    }
}

/// Collect JavaScript/TypeScript local variable declarations
fn collect_js_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "variable_declaration" | "lexical_declaration" => {
            // let/const/var x = ...
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_declarator" {
                    if let Some(name) = child.child_by_field_name("name") {
                        collect_js_binding(name, source, locals);
                    }
                }
            }
        }
        "for_in_statement" | "for_of_statement" => {
            // for (let x in ...) or for (const x of ...)
            if let Some(left) = node.child_by_field_name("left") {
                collect_js_binding(left, source, locals);
            }
        }
        "catch_clause" => {
            // catch (e) { ... }
            if let Some(param) = node.child_by_field_name("parameter") {
                collect_js_binding(param, source, locals);
            }
        }
        "array_pattern" | "object_pattern" => {
            // Destructuring: const [a, b] = ... or const {x, y} = ...
            collect_js_binding(node, source, locals);
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_js_locals(child, source, locals);
            }
        }
    }
}

/// Collect JS binding patterns
fn collect_js_binding(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            locals.insert(node_text(node, source).to_string());
        }
        "array_pattern" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_js_binding(child, source, locals);
            }
        }
        "object_pattern" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "shorthand_property_identifier_pattern" => {
                        locals.insert(node_text(child, source).to_string());
                    }
                    "pair_pattern" => {
                        if let Some(value) = child.child_by_field_name("value") {
                            collect_js_binding(value, source, locals);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// Collect Rust local variable declarations
fn collect_rust_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "let_declaration" => {
            // let x = ...
            if let Some(pattern) = node.child_by_field_name("pattern") {
                collect_rust_pattern(pattern, source, locals);
            }
        }
        "for_expression" => {
            // for x in ...
            if let Some(pattern) = node.child_by_field_name("pattern") {
                collect_rust_pattern(pattern, source, locals);
            }
        }
        "if_let_expression" | "while_let_expression" => {
            // if let Some(x) = ...
            if let Some(pattern) = node.child_by_field_name("pattern") {
                collect_rust_pattern(pattern, source, locals);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_rust_locals(child, source, locals);
            }
        }
    }
}

/// Collect Rust patterns
fn collect_rust_pattern(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            locals.insert(node_text(node, source).to_string());
        }
        "tuple_pattern" | "slice_pattern" | "struct_pattern" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_rust_pattern(child, source, locals);
            }
        }
        "ref_pattern" | "mutable_pattern" => {
            // &x or mut x
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_rust_pattern(child, source, locals);
            }
        }
        _ => {}
    }
}

/// Collect Java local variable declarations
fn collect_java_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "local_variable_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_declarator" {
                    if let Some(name) = child.child_by_field_name("name") {
                        locals.insert(node_text(name, source).to_string());
                    }
                }
            }
        }
        "enhanced_for_statement" => {
            // for (Type x : collection)
            if let Some(name) = node.child_by_field_name("name") {
                locals.insert(node_text(name, source).to_string());
            }
        }
        "catch_clause" => {
            // catch (Exception e)
            if let Some(param) = node.child_by_field_name("parameter") {
                if let Some(name) = param.child_by_field_name("name") {
                    locals.insert(node_text(name, source).to_string());
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_java_locals(child, source, locals);
            }
        }
    }
}

/// Collect C/C++ local variable declarations
fn collect_c_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "declaration" => {
            // int x = 1, y = 2;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "init_declarator" {
                    // Look for declarator which contains the name
                    if let Some(declarator) = child.child_by_field_name("declarator") {
                        collect_c_declarator_name_to_locals(declarator, source, locals);
                    }
                } else if child.kind() == "identifier" {
                    // Simple declaration without init
                    locals.insert(node_text(child, source).to_string());
                }
            }
        }
        "for_statement" => {
            // Check for declaration in for init
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "declaration" {
                    collect_c_locals(child, source, locals);
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_c_locals(child, source, locals);
            }
        }
    }
}

/// Extract name from C declarator and add to locals
fn collect_c_declarator_name_to_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "identifier" | "field_identifier" => {
            locals.insert(node_text(node, source).to_string());
        }
        "pointer_declarator" | "reference_declarator" | "function_declarator" |
        "parenthesized_declarator" | "array_declarator" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                collect_c_declarator_name_to_locals(declarator, source, locals);
            }
        }
        _ => {}
    }
}

/// Collect Ruby local variable declarations
fn collect_ruby_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "assignment" => {
            // x = ...
            if let Some(left) = node.child_by_field_name("left") {
                collect_ruby_lhs(left, source, locals);
            }
        }
        "each" | "for" => {
            // each do |x, y| ... end
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "block_parameters" {
                    collect_ruby_block_params(child, source, locals);
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_ruby_locals(child, source, locals);
            }
        }
    }
}

/// Collect Ruby LHS of assignment
fn collect_ruby_lhs(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            locals.insert(node_text(node, source).to_string());
        }
        _ => {
            // Could be attribute assignment (obj.x = ...) - skip
        }
    }
}

/// Collect Ruby block parameters
fn collect_ruby_block_params(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "block_parameter" {
            if let Some(name) = child.child_by_field_name("name") {
                locals.insert(node_text(name, source).to_string());
            } else if child.kind() == "identifier" {
                locals.insert(node_text(child, source).to_string());
            }
        }
    }
}

/// Collect PHP local variable declarations
fn collect_php_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "assignment_expression" => {
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "variable_name" {
                    if let Some(ident) = left.child(0) {
                        if ident.kind() == "name" {
                            locals.insert(node_text(ident, source).to_string());
                        }
                    }
                }
            }
        }
        "foreach_statement" => {
            // foreach ($items as $key => $value)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_name" {
                    if let Some(ident) = child.child(0) {
                        if ident.kind() == "name" {
                            locals.insert(node_text(ident, source).to_string());
                        }
                    }
                }
            }
        }
        "catch_clause" => {
            // catch (Exception $e)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_name" {
                    if let Some(ident) = child.child(0) {
                        if ident.kind() == "name" {
                            locals.insert(node_text(ident, source).to_string());
                        }
                    }
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_php_locals(child, source, locals);
            }
        }
    }
}

/// Collect Kotlin local variable declarations
fn collect_kotlin_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "property_declaration" => {
            // val/var x = ...
            if let Some(name) = node.child_by_field_name("name") {
                locals.insert(node_text(name, source).to_string());
            }
        }
        "for_statement" => {
            // for (x in ...)
            if let Some(name) = node.child_by_field_name("name") {
                locals.insert(node_text(name, source).to_string());
            }
        }
        "catch_block" => {
            // catch (e: Exception)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "catch_parameter" {
                    if let Some(name) = child.child_by_field_name("name") {
                        locals.insert(node_text(name, source).to_string());
                    }
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_kotlin_locals(child, source, locals);
            }
        }
    }
}

/// Collect Swift local variable declarations
fn collect_swift_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "property_declaration" => {
            // let/var x = ...
            if let Some(name) = node.child_by_field_name("name") {
                locals.insert(node_text(name, source).to_string());
            }
        }
        "for_statement" => {
            // for x in ...
            if let Some(name) = node.child_by_field_name("name") {
                locals.insert(node_text(name, source).to_string());
            }
        }
        "catch_block" => {
            // catch { e in ... }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "catch_parameter" {
                    if let Some(name) = child.child_by_field_name("name") {
                        locals.insert(node_text(name, source).to_string());
                    }
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_swift_locals(child, source, locals);
            }
        }
    }
}

/// Collect C# local variable declarations
fn collect_csharp_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "local_declaration_statement" => {
            // var x = ... or Type x = ...
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_declaration" {
                    let mut decl_cursor = child.walk();
                    for var_decl in child.children(&mut decl_cursor) {
                        if var_decl.kind() == "variable_declarator" {
                            if let Some(name) = var_decl.child_by_field_name("name") {
                                locals.insert(node_text(name, source).to_string());
                            }
                        }
                    }
                }
            }
        }
        "foreach_statement" => {
            // foreach (var x in ...)
            if let Some(name) = node.child_by_field_name("name") {
                locals.insert(node_text(name, source).to_string());
            }
        }
        "catch_clause" => {
            // catch (Exception e)
            if let Some(param) = node.child_by_field_name("declaration") {
                if let Some(name) = param.child_by_field_name("name") {
                    locals.insert(node_text(name, source).to_string());
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_csharp_locals(child, source, locals);
            }
        }
    }
}

/// Collect Scala local variable declarations
fn collect_scala_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "val_definition" | "var_definition" => {
            // val x = ... or var x = ...
            if let Some(name) = node.child_by_field_name("name") {
                locals.insert(node_text(name, source).to_string());
            }
        }
        "for_expression" => {
            // for { x <- ... }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "enumerators" {
                    collect_scala_locals(child, source, locals);
                }
            }
        }
        "enumerator" => {
            // x <- items
            if let Some(name) = node.child_by_field_name("name") {
                locals.insert(node_text(name, source).to_string());
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_scala_locals(child, source, locals);
            }
        }
    }
}

/// Collect Elixir local variable declarations (mostly pattern matches)
fn collect_elixir_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    // Elixir variables are created by pattern matching
    // In do blocks, any identifier on the LHS of = is a local
    match node.kind() {
        "match_expression" | "binary_operator" => {
            // x = ... or x <- ...
            if let Some(left) = node.child_by_field_name("left") {
                collect_elixir_pattern(left, source, locals);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_elixir_locals(child, source, locals);
            }
        }
    }
}

/// Collect Elixir pattern bindings
fn collect_elixir_pattern(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            locals.insert(node_text(node, source).to_string());
        }
        _ => {
            // Could be tuple, list patterns - recurse
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_elixir_pattern(child, source, locals);
            }
        }
    }
}

/// Collect Lua local variable declarations
fn collect_lua_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "local_variable_declaration" => {
            // local x, y = ...
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    locals.insert(node_text(child, source).to_string());
                }
            }
        }
        "assignment_statement" => {
            // x = ... (this could be global or local, we track both and filter later)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_list" {
                    let mut vl_cursor = child.walk();
                    for var in child.children(&mut vl_cursor) {
                        if var.kind() == "identifier" {
                            locals.insert(node_text(var, source).to_string());
                        }
                    }
                }
            }
        }
        "numeric_for_statement" | "generic_for_statement" => {
            // for i = 1, 10 do ... end or for k, v in pairs(t) do ... end
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    locals.insert(node_text(child, source).to_string());
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_lua_locals(child, source, locals);
            }
        }
    }
}

/// Collect OCaml local variable declarations
fn collect_ocaml_locals(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "let_binding" => {
            // let x = ...
            if let Some(name) = node.child_by_field_name("pattern") {
                collect_ocaml_pattern(name, source, locals);
            }
        }
        "value_definition" => {
            // let rec x = ...
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "let_binding" {
                    collect_ocaml_locals(child, source, locals);
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_ocaml_locals(child, source, locals);
            }
        }
    }
}

/// Collect OCaml patterns
fn collect_ocaml_pattern(node: Node, source: &[u8], locals: &mut HashSet<String>) {
    match node.kind() {
        "value_name" | "identifier" => {
            locals.insert(node_text(node, source).to_string());
        }
        "tuple_pattern" | "list_pattern" | "array_pattern" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_ocaml_pattern(child, source, locals);
            }
        }
        _ => {}
    }
}

/// Detect direct effects in a function body
pub fn detect_direct_effects(func_node: Node, source: &[u8], function_names: &HashSet<String>, lang: Language) -> DetectedEffects {
    let mut effects = DetectedEffects::default();
    
    // Extract parameter names and local variable names for scope analysis
    let param_names = extract_parameter_names(func_node, source, lang);
    let local_vars = extract_local_variables(func_node, source, lang);
    
    // Create a scope context that combines parameters and local vars
    let scope = ScopeContext {
        param_names: &param_names,
        local_vars: &local_vars,
        function_names,
    };

    detect_effects_recursive(func_node, source, &scope, &mut effects, 0, lang);

    effects
}

/// Scope context for purity analysis
/// Tracks which variables are parameters vs local variables
struct ScopeContext<'a> {
    /// Parameter names (mutating these is impure)
    param_names: &'a HashSet<String>,
    /// Local variable names (mutating these is pure)
    local_vars: &'a HashSet<String>,
    /// Function names in the current module (for interprocedural)
    function_names: &'a HashSet<String>,
}

impl<'a> ScopeContext<'a> {
    /// Check if a name refers to a parameter (not a local variable)
    /// Mutating parameters is impure; mutating locals is pure
    fn is_parameter_mutation(&self, name: &str) -> bool {
        // If it's in param_names AND not in local_vars, it's a parameter mutation (impure)
        // If it's in local_vars, it's a local mutation (pure)
        // If it's in neither, conservatively treat as potentially impure
        self.param_names.contains(name) && !self.local_vars.contains(name)
    }
    
    /// Check if a name is a known local function
    fn is_local_function(&self, name: &str) -> bool {
        self.function_names.contains(name)
    }
}

/// Recursively detect effects in AST nodes
fn detect_effects_recursive(
    node: Node,
    source: &[u8],
    scope: &ScopeContext,
    effects: &mut DetectedEffects,
    depth: usize,
    lang: Language,
) {
    // Prevent stack overflow on deeply nested code
    if depth > 100 {
        return;
    }

    let mut cursor = node.walk();

    match node.kind() {
        // Global statement: global x
        "global_statement" => {
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    effects.global_writes.push(node_text(child, source).to_string());
                }
            }
        }

        // Nonlocal statement: nonlocal x
        "nonlocal_statement" => {
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    effects.global_writes.push(node_text(child, source).to_string());
                }
            }
        }

        // Assignment: check for attribute writes (self.x = ..., ptr->field = ...)
        // Python: "assignment" | "augmented_assignment"
        // C/C++/Go/Java/JS/TS/Rust: "assignment_expression"
        // Rust: "compound_assignment_expr"
        // JS/TS: "augmented_assignment_expression"
        "assignment" | "augmented_assignment"
        | "assignment_expression"
        | "compound_assignment_expr"
        | "augmented_assignment_expression" => {
            if let Some(left) = node.child_by_field_name("left") {
                check_attribute_write(left, source, scope, effects);
            }
            // Also check children for simple assignments
            for child in node.children(&mut cursor) {
                // Check for field/attribute writes across languages:
                // - Python: "attribute" (self.field = value)
                // - C/C++/Rust: "field_expression" (ptr->field = value, self.field = value)
                // - Go: "selector_expression" (receiver.field = value)
                // - JS/TS: "member_expression" (this.field = value, obj.prop = value)
                // - Java: "field_access" (this.field = value)
                if matches!(child.kind(),
                    "attribute" | "field_expression" | "selector_expression" |
                    "member_expression" | "field_access") {
                    check_attribute_write(child, source, scope, effects);
                    break; // Only check left side
                }
            }
        }

        // Call expression: covers Python ("call"), Go/JS/TS/C/C++/Kotlin/Scala/Swift ("call_expression"),
        // C# ("invocation_expression"), PHP ("function_call_expression", "member_call_expression"),
        // Lua ("function_call"), OCaml ("application_expression"), Ruby ("command" for bare calls)
        "call" | "call_expression"
        | "invocation_expression"
        | "function_call_expression" | "member_call_expression"
        | "function_call"
        | "application_expression"
        | "command" => {
            check_call_effects(node, source, scope, effects, lang);
        }

        // Java/Kotlin method invocation: System.out.println(...)
        "method_invocation" => {
            check_call_effects_method_invocation(node, source, scope, effects, lang);
        }

        // Rust macro invocation: println!(...), eprintln!(...)
        "macro_invocation" => {
            check_call_effects_macro(node, source, effects, lang);
        }

        // PHP echo statement: `echo $x;` (not a call node, but an I/O statement)
        "echo_statement" => {
            effects.io_operations.push("echo".to_string());
        }

        // Ruby/Elixir: bare identifiers can be method calls (e.g., `rand`, `exit`)
        // Only check if the identifier text matches a known impure/IO call for this language
        "identifier" if matches!(lang, Language::Ruby | Language::Elixir) => {
            let name = node_text(node, source);
            let is_io = io_ops_for_language(lang).contains(&name);
            let is_impure = impure_calls_for_language(lang).contains(&name);
            if is_io || is_impure {
                effects.io_operations.push(name.to_string());
            }
        }

        _ => {}
    }

    // Recurse into children
    let mut child_cursor = node.walk();
    for child in node.children(&mut child_cursor) {
        detect_effects_recursive(child, source, scope, effects, depth + 1, lang);
    }
}

/// Check if an assignment target is an attribute write (self.x = ..., ptr->field = ...)
///
/// Handles field/attribute mutation across multiple languages:
/// - Python: `attribute` nodes with `object`/`attribute` fields
/// - C/C++/Rust: `field_expression` nodes with `argument`/`field` or `value`/`field` fields
/// - Go: `selector_expression` nodes with `operand`/`field` fields
/// - JavaScript/TypeScript: `member_expression` nodes with `object`/`property` fields
/// - Java: `field_access` nodes with `object`/`field` fields
///
/// NOTE: Mutating local struct fields (e.g., `local.field = x`) is PURE
/// because it doesn't affect state outside the function.
/// Only receiver field mutations (self.x, s.x where s is a parameter) are impure.
fn check_attribute_write(node: Node, source: &[u8], scope: &ScopeContext, effects: &mut DetectedEffects) {
    /// Helper: check if an object name refers to a non-local (impure) target
    fn is_impure_target(obj_text: &str, scope: &ScopeContext) -> bool {
        let is_self = obj_text == "self" || obj_text == "Self" || obj_text == "this";
        let is_param_mutation = scope.is_parameter_mutation(obj_text);
        // If it's in local_vars and NOT in param_names, it's a local mutation (pure)
        let is_local = scope.local_vars.contains(obj_text) && !scope.param_names.contains(obj_text);
        (is_self || is_param_mutation || !is_local) && !obj_text.is_empty()
    }

    match node.kind() {
        // Python: self.field = value
        "attribute" => {
            if let Some(obj) = node.child_by_field_name("object") {
                let obj_text = node_text(obj, source);
                if is_impure_target(obj_text, scope) {
                    if let Some(attr) = node.child_by_field_name("attribute") {
                        let attr_text = node_text(attr, source);
                        effects.attribute_writes.push(format!("{}.{}", obj_text, attr_text));
                    }
                }
            }
        }

        // C/C++/Rust: ptr->field = value or self.field = value
        // Tree-sitter field_expression has different field names depending on language:
        // - C/C++: `argument` (struct) and `field` (member)
        // - Rust: `value` and `field`
        "field_expression" => {
            // Try C/C++ field names first
            let obj = node
                .child_by_field_name("argument")
                .or_else(|| node.child_by_field_name("value"))
                .or_else(|| node.child(0)); // Fallback to first child
            let field = node
                .child_by_field_name("field")
                .or_else(|| node.child_by_field_name("field_identifier"));

            if let (Some(obj), Some(field)) = (obj, field) {
                let obj_text = extract_name_from_expr(obj, source);
                let field_text = node_text(field, source);
                if is_impure_target(&obj_text, scope) {
                    effects.attribute_writes.push(format!("{}.{}", obj_text, field_text));
                }
            } else if let Some(obj) = obj {
                let obj_text = extract_name_from_expr(obj, source);
                if is_impure_target(&obj_text, scope) {
                    let expr_text = extract_name_from_expr(node, source);
                    effects.attribute_writes.push(expr_text);
                }
            }
        }

        // Go: receiver.field = value
        "selector_expression" => {
            if let Some(operand) = node.child_by_field_name("operand") {
                let obj_text = extract_name_from_expr(operand, source);
                if is_impure_target(&obj_text, scope) {
                    if let Some(field) = node.child_by_field_name("field") {
                        let field_text = node_text(field, source);
                        effects.attribute_writes.push(format!("{}.{}", obj_text, field_text));
                    } else {
                        effects.attribute_writes.push(obj_text);
                    }
                }
            }
        }

        // JavaScript/TypeScript: this.field = value, obj.prop = value
        "member_expression" => {
            if let Some(obj) = node.child_by_field_name("object") {
                let obj_text = extract_name_from_expr(obj, source);
                if is_impure_target(&obj_text, scope) {
                    if let Some(prop) = node.child_by_field_name("property") {
                        let prop_text = node_text(prop, source);
                        effects.attribute_writes.push(format!("{}.{}", obj_text, prop_text));
                    } else {
                        effects.attribute_writes.push(obj_text);
                    }
                }
            }
        }

        // Java: this.field = value
        "field_access" => {
            if let Some(obj) = node.child_by_field_name("object") {
                let obj_text = extract_name_from_expr(obj, source);
                if is_impure_target(&obj_text, scope) {
                    if let Some(field) = node.child_by_field_name("field") {
                        let field_text = node_text(field, source);
                        effects.attribute_writes.push(format!("{}.{}", obj_text, field_text));
                    } else {
                        effects.attribute_writes.push(obj_text);
                    }
                }
            }
        }

        _ => {}
    }
}

/// Check a call expression for effects
fn check_call_effects(
    node: Node,
    source: &[u8],
    scope: &ScopeContext,
    effects: &mut DetectedEffects,
    lang: Language,
) {
    // Track that we encountered a function call (including pure builtins).
    // This distinguishes "no calls at all" from "only known-pure calls".
    effects.has_any_calls = true;

    let call_name = extract_call_name(node, source);
    let call_str = call_name.as_deref().unwrap_or("");

    // Check for I/O operations (language-specific)
    for &io_op in io_ops_for_language(lang) {
        if call_str == io_op || call_str.ends_with(&format!(".{}", io_op)) {
            effects.io_operations.push(call_str.to_string());
            return;
        }
    }

    // Check for known impure calls (language-specific)
    for &impure in impure_calls_for_language(lang) {
        if call_str == impure || call_str.ends_with(impure) {
            effects.io_operations.push(call_str.to_string());
            return;
        }
    }

    // Check for collection mutations (language-specific)
    let method_name = call_str.split('.').last().unwrap_or("");
    for &mutation in collection_mutations_for_language(lang) {
        if method_name == mutation {
            // Check if the base object is a local variable vs parameter
            // e.g., local_list.append(x) -> pure (local mutation)
            //       param_list.append(x) -> impure (parameter mutation)
            //       self.list.append(x) -> impure (receiver mutation)
            let base_obj = call_str.split('.').next().unwrap_or("");
            
            // If base object is a local variable (not a parameter), skip - local mutation is pure
            if scope.local_vars.contains(base_obj) && !scope.param_names.contains(base_obj) {
                // This is a local variable mutation - pure, skip
                return;
            }
            
            // If base object is a parameter (and not shadowed by a local), it's impure
            // Also receiver mutations (self) are impure
            if scope.param_names.contains(base_obj) || base_obj == "self" || base_obj == "Self" {
                effects.collection_mutations.push(call_str.to_string());
                return;
            }
            
            // For builtin functions like Go's append(), check if the first argument is a local
            // Go: append(suggestions, x) where suggestions is local -> pure
            if is_builtin_mutation_function(method_name, lang) {
                if let Some(first_arg) = get_first_call_argument(node) {
                    let arg_name = extract_identifier_name(first_arg, source);
                    if scope.local_vars.contains(&arg_name) && !scope.param_names.contains(&arg_name) {
                        // First argument is a local variable - pure mutation
                        return;
                    }
                }
            }
            
            // For unknown base objects, conservatively mark as impure
            // This handles cases like global variable mutations
            effects.collection_mutations.push(call_str.to_string());
            return;
        }
    }

    // Track local calls for interprocedural analysis
    let base_name = call_str.split('.').next().unwrap_or(call_str);
    if scope.is_local_function(base_name) {
        effects.local_calls.push(base_name.to_string());
    } else if !is_known_pure_builtin(call_str) && !call_str.is_empty() {
        // Unknown external call - might have effects
        effects.unknown_calls.push(call_str.to_string());
    }
}

/// Check if a function name is a builtin mutation function (like Go's append)
fn is_builtin_mutation_function(name: &str, lang: Language) -> bool {
    match lang {
        Language::Go => matches!(name, "append" | "copy"),
        Language::Rust => matches!(name, "push" | "pop" | "insert" | "remove"),
        _ => false,
    }
}

/// Get the first argument of a call expression
fn get_first_call_argument(node: Node) -> Option<Node> {
    // Try "arguments" field first (common in many languages)
    if let Some(args) = node.child_by_field_name("arguments") {
        let mut cursor = args.walk();
        for child in args.children(&mut cursor) {
            // Skip parentheses and commas
            if !matches!(child.kind(), "(" | ")" | ",") {
                return Some(child);
            }
        }
    }
    None
}

/// Extract a simple identifier name from a node
fn extract_identifier_name(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "identifier" | "simple_identifier" | "value_name" => {
            node_text(node, source).to_string()
        }
        // For pointer dereference like *items in Go, look at the operand
        "unary_expression" | "pointer_expression" => {
            if let Some(operand) = node.child_by_field_name("operand") {
                extract_identifier_name(operand, source)
            } else {
                String::new()
            }
        }
        // For selector expressions like s.handlers, return the base object
        "selector_expression" => {
            if let Some(operand) = node.child_by_field_name("operand") {
                extract_identifier_name(operand, source)
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// Check a Java/Kotlin method_invocation node for effects.
///
/// Java method_invocation has `object` + `name` fields, e.g.:
///   System.out.println("hello") -> object=field_access(System.out), name=println
fn check_call_effects_method_invocation(
    node: Node,
    source: &[u8],
    scope: &ScopeContext,
    effects: &mut DetectedEffects,
    lang: Language,
) {
    effects.has_any_calls = true;

    // Build the full dotted name: object.name
    let mut parts = Vec::new();

    if let Some(obj) = node.child_by_field_name("object") {
        parts.push(extract_name_from_expr(obj, source));
    }
    if let Some(name) = node.child_by_field_name("name") {
        parts.push(node_text(name, source).to_string());
    }

    let call_str = parts.join(".");

    // Check against language-specific IO operations
    for &io_op in io_ops_for_language(lang) {
        if call_str == io_op || call_str.ends_with(&format!(".{}", io_op)) {
            effects.io_operations.push(call_str);
            return;
        }
    }

    // Check against language-specific impure calls
    for &impure in impure_calls_for_language(lang) {
        if call_str == impure || call_str.ends_with(impure) {
            effects.io_operations.push(call_str);
            return;
        }
    }

    // Check collection mutations
    let method_name = call_str.split('.').last().unwrap_or("");
    for &mutation in collection_mutations_for_language(lang) {
        if method_name == mutation {
            // Check if mutating a local variable (pure) vs parameter/receiver (impure)
            let base_obj = call_str.split('.').next().unwrap_or("");
            
            // Local variable mutation is pure
            if scope.local_vars.contains(base_obj) && !scope.param_names.contains(base_obj) {
                return;
            }
            
            // Parameter or receiver mutation is impure
            effects.collection_mutations.push(call_str);
            return;
        }
    }

    // Track local or unknown calls
    let base_name = call_str.split('.').next().unwrap_or(&call_str);
    if scope.is_local_function(base_name) {
        effects.local_calls.push(base_name.to_string());
    } else if !call_str.is_empty() {
        effects.unknown_calls.push(call_str);
    }
}

/// Check a Rust macro_invocation node for effects.
///
/// Rust macro_invocation has a `macro` field with the macro name (e.g., "println").
/// We append "!" to form "println!" and check against Rust IO operations.
fn check_call_effects_macro(
    node: Node,
    source: &[u8],
    effects: &mut DetectedEffects,
    lang: Language,
) {
    effects.has_any_calls = true;

    let macro_name = if let Some(m) = node.child_by_field_name("macro") {
        let name = node_text(m, source).to_string();
        format!("{}!", name)
    } else {
        return;
    };

    // Check against language-specific IO operations
    for &io_op in io_ops_for_language(lang) {
        if macro_name == io_op {
            effects.io_operations.push(macro_name);
            return;
        }
    }

    // Check against language-specific impure calls
    for &impure in impure_calls_for_language(lang) {
        if macro_name == impure {
            effects.io_operations.push(macro_name);
            return;
        }
    }
}

/// Extract the full call name from a call node
fn extract_call_name(node: Node, source: &[u8]) -> Option<String> {
    // Try field-based extraction first (Python/JS/TS/Go/C/C++: "function" field)
    if let Some(func) = node.child_by_field_name("function") {
        return Some(extract_name_from_expr(func, source));
    }

    // Ruby command: method name via "name" or "method" field
    if let Some(method) = node.child_by_field_name("method") {
        return Some(node_text(method, source).to_string());
    }
    if let Some(name) = node.child_by_field_name("name") {
        return Some(node_text(name, source).to_string());
    }

    // Elixir: "target" field for qualified calls like IO.puts
    if let Some(target) = node.child_by_field_name("target") {
        return Some(extract_name_from_expr(target, source));
    }

    // Fallback: iterate children and match known expression kinds
    for child in node.children(&mut node.walk()) {
        match child.kind() {
            // Standard identifiers
            "identifier" | "simple_identifier" => {
                return Some(node_text(child, source).to_string());
            }
            // Dotted access patterns (Python, Go, JS, Java, C#, etc.)
            "attribute" | "selector_expression" | "member_expression" | "field_access"
            | "member_access_expression" | "navigation_expression" | "scoped_identifier" => {
                return Some(extract_name_from_expr(child, source));
            }
            // Elixir: dot node for qualified calls (IO.puts)
            "dot" => {
                return Some(extract_name_from_expr(child, source));
            }
            // OCaml: value_path wrapping value_name
            "value_path" => {
                return Some(extract_name_from_expr(child, source));
            }
            _ => continue,
        }
    }
    None
}

/// Extract a dotted name from an expression (e.g., a.b.c)
///
/// Handles multiple AST representations of dotted names:
/// - Python: `attribute` nodes with `object`/`attribute` fields
/// - Go: `selector_expression` nodes with `operand`/`field` fields
/// - JS/TS: `member_expression` nodes with `object`/`property` fields
/// - Java: `field_access` nodes with `object`/`field` fields
fn extract_name_from_expr(node: Node, source: &[u8]) -> String {
    match node.kind() {
        // Simple identifiers (various languages use different kinds)
        "identifier" | "field_identifier" | "property_identifier"
        | "simple_identifier"  // Swift, Kotlin
        | "value_name" => {     // OCaml
            node_text(node, source).to_string()
        }

        // Python: obj.attr
        "attribute" => {
            extract_dotted_name(node, source, "object", "attribute")
        }

        // Go: pkg.Func (selector_expression with operand/field)
        "selector_expression" => {
            extract_dotted_name(node, source, "operand", "field")
        }

        // JS/TS: obj.method (member_expression with object/property)
        "member_expression" => {
            extract_dotted_name(node, source, "object", "property")
        }

        // Java: Obj.field (field_access with object/field)
        "field_access" => {
            extract_dotted_name(node, source, "object", "field")
        }

        // C#: Console.WriteLine (member_access_expression with children identifier.identifier)
        "member_access_expression" => {
            // Try field-based extraction first: expression.name
            if let Some(name) = node.child_by_field_name("name") {
                if let Some(expr) = node.child_by_field_name("expression") {
                    let obj_name = extract_name_from_expr(expr, source);
                    let attr_name = node_text(name, source);
                    return format!("{}.{}", obj_name, attr_name);
                }
            }
            // Fallback: collect identifiers joined by "."
            extract_children_dotted(node, source)
        }

        // C#: new Random() in object_creation_expression
        "object_creation_expression" => {
            // Extract the type name from the object_creation_expression
            // e.g., "new Random()" -> extract "Random"
            if let Some(type_node) = node.child_by_field_name("type") {
                return node_text(type_node, source).to_string();
            }
            node_text(node, source).to_string()
        }

        // Swift: navigation_expression (expression.suffix)
        "navigation_expression" => {
            if let Some(suffix) = node.child_by_field_name("suffix") {
                let suffix_text = node_text(suffix, source);
                // Get the expression (left side)
                for child in node.children(&mut node.walk()) {
                    if child.id() != suffix.id() && child.kind() != "." {
                        let obj_name = extract_name_from_expr(child, source);
                        return format!("{}.{}", obj_name, suffix_text);
                    }
                }
                return suffix_text.to_string();
            }
            node_text(node, source).to_string()
        }

        // Elixir: dot node for qualified calls (IO.puts => alias.identifier)
        "dot" => {
            extract_children_dotted(node, source)
        }

        // C#: scoped_identifier (System.Console etc.)
        "scoped_identifier" => {
            extract_children_dotted(node, source)
        }

        // OCaml: value_path wrapping value_name
        "value_path" => {
            // value_path can contain module_path.value_name
            let mut parts = Vec::new();
            for child in node.children(&mut node.walk()) {
                match child.kind() {
                    "value_name" | "module_name" | "module_path" => {
                        parts.push(extract_name_from_expr(child, source));
                    }
                    _ => {}
                }
            }
            if parts.is_empty() {
                node_text(node, source).to_string()
            } else {
                parts.join(".")
            }
        }

        // OCaml: module_path for qualified names (Random.int -> module_name.value_name)
        "module_path" => {
            node_text(node, source).to_string()
        }

        // Elixir: alias node (IO, MyModule)
        "alias" => {
            node_text(node, source).to_string()
        }

        // PHP: name node used in function_call_expression
        "name" => {
            node_text(node, source).to_string()
        }

        _ => node_text(node, source).to_string(),
    }
}

/// Extract a dotted name by collecting identifier-like children separated by "."
fn extract_children_dotted(node: Node, source: &[u8]) -> String {
    let mut parts = Vec::new();
    for child in node.children(&mut node.walk()) {
        match child.kind() {
            "identifier" | "simple_identifier" | "alias" | "name"
            | "module_name" | "value_name" => {
                parts.push(node_text(child, source).to_string());
            }
            "." | "?" => {} // skip punctuation
            _ => {
                // Recurse for nested expressions
                let sub = extract_name_from_expr(child, source);
                if !sub.is_empty() && sub != "." {
                    parts.push(sub);
                }
            }
        }
    }
    parts.join(".")
}

/// Extract a dotted name by walking a chain of dotted-access nodes.
///
/// `obj_field` is the field name for the object/left side (e.g. "object", "operand").
/// `attr_field` is the field name for the attribute/right side (e.g. "attribute", "field", "property").
fn extract_dotted_name(node: Node, source: &[u8], obj_field: &str, attr_field: &str) -> String {
    let mut parts = Vec::new();

    // Get the right-side name (method/field/property)
    if let Some(attr) = node.child_by_field_name(attr_field) {
        parts.push(node_text(attr, source).to_string());
    }

    // Get the left-side (object/operand) and recurse if it's also a dotted expression
    if let Some(obj) = node.child_by_field_name(obj_field) {
        let obj_name = extract_name_from_expr(obj, source);
        if !obj_name.is_empty() {
            parts.insert(0, obj_name);
        }
    }

    parts.join(".")
}

/// Check if a function name is a known pure builtin
fn is_known_pure_builtin(name: &str) -> bool {
    const PURE_BUILTINS: &[&str] = &[
        "len", "range", "int", "float", "str", "bool", "list", "dict", "set",
        "tuple", "sorted", "reversed", "enumerate", "zip", "map", "filter",
        "min", "max", "sum", "abs", "round", "isinstance", "issubclass",
        "type", "id", "hash", "repr", "next", "iter", "all", "any",
        "chr", "ord", "hex", "oct", "bin", "pow", "divmod",
        "super", "property", "staticmethod", "classmethod",
        "frozenset", "bytes", "bytearray", "memoryview", "complex",
        "slice", "object", "format", "ascii", "callable", "hasattr", "getattr",
        // String methods
        "upper", "lower", "strip", "split", "join", "replace", "format",
        "startswith", "endswith", "encode", "decode",
        // Math
        "math.sqrt", "math.sin", "math.cos", "math.tan", "math.log", "math.exp",
        "math.floor", "math.ceil", "math.abs", "math.pow",
    ];

    let base = name.split('.').last().unwrap_or(name);
    PURE_BUILTINS.contains(&name) || PURE_BUILTINS.contains(&base)
}

// =============================================================================
// Interprocedural Analysis
// =============================================================================

/// Propagate impurity through the call graph
pub fn propagate_impurity(analysis: &mut EffectAnalysis) {
    // Build reverse mapping: who calls whom
    let mut callers: HashMap<String, Vec<String>> = HashMap::new();
    for (caller, callees) in &analysis.call_graph {
        for callee in callees {
            callers.entry(callee.clone()).or_default().push(caller.clone());
        }
    }

    // Iterative propagation until fixpoint
    let mut changed = true;
    let mut iterations = 0;
    let max_iterations = analysis.purity.len() + 10;

    while changed && iterations < max_iterations {
        changed = false;
        iterations += 1;

        // For each impure function, mark its callers as impure
        let impure_funcs: Vec<String> = analysis
            .purity
            .iter()
            .filter(|(_, &is_pure)| !is_pure)
            .map(|(name, _)| name.clone())
            .collect();

        for impure_func in impure_funcs {
            if let Some(caller_list) = callers.get(&impure_func) {
                for caller in caller_list {
                    if let Some(is_pure) = analysis.purity.get_mut(caller) {
                        if *is_pure {
                            *is_pure = false;
                            changed = true;

                            // Also propagate effects
                            if let Some(callee_effects) = analysis.effects.get(&impure_func).cloned() {
                                if let Some(caller_effects) = analysis.effects.get_mut(caller) {
                                    // Merge effects
                                    caller_effects.io_operations.extend(callee_effects.io_operations);
                                    caller_effects.global_writes.extend(callee_effects.global_writes);
                                    caller_effects.attribute_writes.extend(callee_effects.attribute_writes);
                                    caller_effects.collection_mutations.extend(callee_effects.collection_mutations);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// =============================================================================
// Confidence Calculation
// =============================================================================

/// Calculate confidence level based on analysis.
///
/// Confidence reflects how much evidence we have for the classification:
/// - Impure with detected effects -> High (we have concrete evidence)
/// - Unknown due to unresolvable calls -> Medium/Low (depending on count)
/// - Pure with all calls resolved -> High (positive evidence)
/// - Unknown due to no calls at all -> Low (no evidence either way)
pub fn confidence_from_analysis(effects: &DetectedEffects) -> Confidence {
    if !effects.unknown_calls.is_empty() {
        // Has calls to unknown functions - lower confidence
        if effects.unknown_calls.len() > 3 {
            Confidence::Low
        } else {
            Confidence::Medium
        }
    } else if effects.has_any_calls || !effects.local_calls.is_empty() {
        // All calls resolved to known-pure builtins or local functions
        Confidence::High
    } else {
        // No calls detected — no evidence for purity claim
        Confidence::Low
    }
}

// =============================================================================
// File Analysis
// =============================================================================

/// Analyze purity for a single file
pub fn analyze_purity_file(path: &Path, args: &PurityArgs) -> PatternsResult<FilePurityReport> {
    // Validate path
    let canonical = if let Some(ref root) = args.project_root {
        validate_file_path_in_project(path, root)?
    } else {
        validate_file_path(path)?
    };

    // Detect language from file extension (default to Python for backward compat)
    let lang = Language::from_path(&canonical).unwrap_or(Language::Python);

    // Read source
    let source = read_file_safe(&canonical)?;
    let source_bytes = source.as_bytes();

    // Parse with tree-sitter (multi-language)
    let mut parser = get_parser_for_language(lang)?;
    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| PatternsError::parse_error(&canonical, "Failed to parse file"))?;

    let root = tree.root_node();
    let func_kinds = function_kinds_for_language(lang);

    // First pass: collect all function names
    let mut function_names: HashSet<String> = HashSet::new();
    collect_function_names(root, source_bytes, &mut function_names, func_kinds);

    // Second pass: analyze each function
    let mut analysis = EffectAnalysis::new();
    analyze_functions(root, source_bytes, &function_names, &mut analysis, func_kinds, lang);

    // Interprocedural propagation
    if !args.no_interprocedural {
        propagate_impurity(&mut analysis);
    }

    // Build results - maintain order by iterating analysis results
    let mut functions = Vec::new();
    for name in analysis.purity.keys() {
        let effects = analysis.effects.get(name).cloned().unwrap_or_default();
        let classification = effects.classification();
        let confidence = confidence_from_analysis(&effects);
        let effect_strings = if classification == "pure" {
            Vec::new()
        } else {
            effects.to_effect_strings()
        };

        functions.push(FunctionPurity {
            name: name.clone(),
            classification: classification.to_string(),
            effects: effect_strings,
            confidence,
        });
    }

    // Sort by name for consistent output
    functions.sort_by(|a, b| a.name.cmp(&b.name));

    // Filter to specific function if requested
    if let Some(ref target) = args.function {
        functions.retain(|f| f.name == *target);
        if functions.is_empty() {
            return Err(PatternsError::function_not_found(target, &canonical));
        }
    }

    let pure_count = functions.iter().filter(|f| f.classification == "pure").count() as u32;

    Ok(FilePurityReport {
        source_file: canonical.to_string_lossy().to_string(),
        functions,
        pure_count,
    })
}

/// For Elixir: check if a `call` node is a `def`/`defp` and extract the function name.
/// Returns Some(name) if this is a def/defp call, None otherwise.
fn elixir_def_name(node: Node, source: &[u8]) -> Option<String> {
    if node.kind() != "call" {
        return None;
    }
    // First child should be identifier "def" or "defp"
    let first_child = node.child(0)?;
    if first_child.kind() != "identifier" {
        return None;
    }
    let keyword = node_text(first_child, source);
    // Skip defmodule - only handle def and defp
    if keyword == "defmodule" {
        return None;
    }
    if keyword != "def" && keyword != "defp" {
        return None;
    }
    // The function name is in the arguments: either a bare identifier or a call node
    // e.g. `def pure_add(a, b) do ... end` -> arguments contain call [pure_add(a, b)]
    // e.g. `def impure_random do ... end` -> arguments contain identifier [impure_random]
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "arguments" {
            let mut arg_cursor = child.walk();
            for arg in child.children(&mut arg_cursor) {
                match arg.kind() {
                    "identifier" => return Some(node_text(arg, source).to_string()),
                    "call" => {
                        // call [impure_print(x)] -> first identifier child is the name
                        let mut call_cursor = arg.walk();
                        for call_child in arg.children(&mut call_cursor) {
                            if call_child.kind() == "identifier" {
                                return Some(node_text(call_child, source).to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

/// Collect function names from the AST (first pass)
fn collect_function_names(node: Node, source: &[u8], function_names: &mut HashSet<String>, func_kinds: &[&str]) {
    // Elixir: detect def/defp calls and extract function name
    if let Some(name) = elixir_def_name(node, source) {
        function_names.insert(name);
    } else if func_kinds.contains(&node.kind()) {
        if let Some(name) = get_function_name(node, source) {
            function_names.insert(name);
        }
    }

    // Handle arrow functions in variable declarations (TS/JS: const f = () => ...)
    if matches!(node.kind(), "lexical_declaration" | "variable_declaration") {
        let mut cur = node.walk();
        for child in node.children(&mut cur) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Some(value_node) = child.child_by_field_name("value") {
                        if matches!(value_node.kind(), "arrow_function" | "function" | "function_expression" | "generator_function") {
                            let var_name = node_text(name_node, source).to_string();
                            if !var_name.is_empty() {
                                function_names.insert(var_name);
                            }
                        }
                    }
                }
            }
        }
    }

    // Recurse into children (for nested functions and methods)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_function_names(child, source, function_names, func_kinds);
    }
}

/// Analyze functions from the AST (second pass)
fn analyze_functions(
    node: Node,
    source: &[u8],
    function_names: &HashSet<String>,
    analysis: &mut EffectAnalysis,
    func_kinds: &[&str],
    lang: Language,
) {
    // Elixir: detect def/defp calls and analyze the do_block for effects
    if let Some(name) = elixir_def_name(node, source) {
        // Find the do_block child to analyze for effects (the function body)
        let mut do_block_node = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "do_block" {
                do_block_node = Some(child);
                break;
            }
        }
        if let Some(body) = do_block_node {
            let effects = detect_direct_effects(body, source, function_names, lang);
            analysis.call_graph.insert(name.clone(), effects.local_calls.clone());
            let is_pure = effects.is_empty();
            analysis.purity.insert(name.clone(), is_pure);
            analysis.effects.insert(name, effects);
        } else {
            // No do_block (one-liner or guard clause) - analyze the whole node
            let effects = detect_direct_effects(node, source, function_names, lang);
            analysis.call_graph.insert(name.clone(), effects.local_calls.clone());
            let is_pure = effects.is_empty();
            analysis.purity.insert(name.clone(), is_pure);
            analysis.effects.insert(name, effects);
        }
        // For Elixir def/defp: only recurse into the do_block for nested function defs,
        // NOT into arguments (which contains a `call` node that would overwrite our analysis)
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            if child.kind() == "do_block" {
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    analyze_functions(inner, source, function_names, analysis, func_kinds, lang);
                }
            }
        }
        return;
    }

    // For Elixir, skip defmodule calls (they're not functions)
    if lang == Language::Elixir && node.kind() == "call" {
        if let Some(first_child) = node.child(0) {
            if first_child.kind() == "identifier" {
                let keyword = node_text(first_child, source);
                if keyword == "defmodule" {
                    // Just recurse into children but don't treat as a function
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        analyze_functions(child, source, function_names, analysis, func_kinds, lang);
                    }
                    return;
                }
            }
        }
    }

    if func_kinds.contains(&node.kind()) {
        if let Some(name) = get_function_name(node, source) {
            let effects = detect_direct_effects(node, source, function_names, lang);

            // Build call graph from local calls
            analysis.call_graph.insert(name.clone(), effects.local_calls.clone());

            // Initial purity based on direct effects
            let is_pure = effects.is_empty();
            analysis.purity.insert(name.clone(), is_pure);
            analysis.effects.insert(name, effects);
        }
    }

    // Handle arrow functions in variable declarations (TS/JS: const f = () => ...)
    if matches!(node.kind(), "lexical_declaration" | "variable_declaration") {
        let mut cur = node.walk();
        for child in node.children(&mut cur) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Some(value_node) = child.child_by_field_name("value") {
                        if matches!(value_node.kind(), "arrow_function" | "function" | "function_expression" | "generator_function") {
                            let var_name = node_text(name_node, source).to_string();
                            if !var_name.is_empty() {
                                let effects = detect_direct_effects(value_node, source, function_names, lang);
                                analysis.call_graph.insert(var_name.clone(), effects.local_calls.clone());
                                let is_pure = effects.is_empty();
                                analysis.purity.insert(var_name.clone(), is_pure);
                                analysis.effects.insert(var_name, effects);
                            }
                        }
                    }
                }
            }
        }
    }

    // Recurse into children (for nested functions and methods)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        analyze_functions(child, source, function_names, analysis, func_kinds, lang);
    }
}

// =============================================================================
// Directory Analysis
// =============================================================================

/// Analyze purity for all Python files in a directory
pub fn analyze_purity_directory(path: &Path, args: &PurityArgs) -> PatternsResult<PurityReport> {
    let canonical = validate_directory_path(path)?;

    let mut files = Vec::new();
    let mut file_count = 0u32;

    // Walk directory
    for entry in walkdir::WalkDir::new(&canonical)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let entry_path = entry.path();

        // Skip files without a recognized language
        if Language::from_path(entry_path).is_none() {
            continue;
        }

        // Skip test files unless --include-tests
        if !args.include_tests {
            let path_str = entry_path.to_string_lossy();
            let file_name = entry_path.file_name().map_or("", |n| n.to_str().unwrap_or(""));
            if file_name.starts_with("test_")
                || path_str.contains("/tests/")
                || path_str.contains("/test/")
                || path_str.contains("\\tests\\")
                || path_str.contains("\\test\\")
            {
                continue;
            }
        }

        // Check file count limit
        file_count += 1;
        check_directory_file_count(file_count as usize)?;

        // Analyze file
        match analyze_purity_file(entry_path, args) {
            Ok(report) => files.push(report),
            Err(_) => continue, // Skip files that fail to parse
        }
    }

    // Calculate totals
    let total_functions: u32 = files.iter().map(|f| f.functions.len() as u32).sum();
    let total_pure: u32 = files.iter().map(|f| f.pure_count).sum();

    Ok(PurityReport {
        files,
        total_functions,
        total_pure,
    })
}

// =============================================================================
// Text Formatting
// =============================================================================

/// Format a purity report as human-readable text
pub fn format_purity_text(report: &PurityReport) -> String {
    let mut lines = Vec::new();

    for file_report in &report.files {
        lines.push(format!("File: {}", file_report.source_file));
        lines.push(String::new());

        for func in &file_report.functions {
            let effects_str = if func.effects.is_empty() {
                "none".to_string()
            } else {
                func.effects.join(", ")
            };

            lines.push(format!("  Function: {}", func.name));
            lines.push(format!("    Classification: {}", func.classification));
            lines.push(format!("    Effects: {}", effects_str));
            lines.push(format!("    Confidence: {}", func.confidence));
            lines.push(String::new());
        }

        lines.push(format!(
            "  Pure functions: {}/{}",
            file_report.pure_count,
            file_report.functions.len()
        ));
        lines.push(String::new());
    }

    lines.push(format!("Total: {} functions, {} pure", report.total_functions, report.total_pure));

    lines.join("\n")
}

// =============================================================================
// Entry Point
// =============================================================================

/// Execute the purity command
pub fn run(args: PurityArgs) -> anyhow::Result<()> {
    let path = &args.path;

    let report = if path.is_dir() {
        analyze_purity_directory(path, &args)?
    } else {
        let file_report = analyze_purity_file(path, &args)?;
        PurityReport {
            total_functions: file_report.functions.len() as u32,
            total_pure: file_report.pure_count,
            files: vec![file_report],
        }
    };

    match args.output_format {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&report)?;
            println!("{}", json);
        }
        OutputFormat::Text => {
            println!("{}", format_purity_text(&report));
        }
    }

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{tempdir, NamedTempFile};

    #[test]
    fn test_io_operations_constant() {
        assert!(IO_OPERATIONS.contains(&"print"));
        assert!(IO_OPERATIONS.contains(&"open"));
        assert!(IO_OPERATIONS.contains(&"read"));
        assert!(IO_OPERATIONS.contains(&"write"));
        assert!(IO_OPERATIONS.contains(&"input"));
    }

    #[test]
    fn test_impure_calls_constant() {
        assert!(IMPURE_CALLS.contains(&"random.random"));
        assert!(IMPURE_CALLS.contains(&"time.time"));
        assert!(IMPURE_CALLS.contains(&"os.system"));
    }

    #[test]
    fn test_is_known_pure_builtin() {
        assert!(is_known_pure_builtin("len"));
        assert!(is_known_pure_builtin("range"));
        assert!(is_known_pure_builtin("sorted"));
        assert!(!is_known_pure_builtin("print"));
        assert!(!is_known_pure_builtin("random"));
    }

    #[test]
    fn test_detected_effects_is_empty() {
        let effects = DetectedEffects::default();
        assert!(effects.is_empty());

        let mut effects2 = DetectedEffects::default();
        effects2.io_operations.push("print".to_string());
        assert!(!effects2.is_empty());
    }

    #[test]
    fn test_detected_effects_to_strings() {
        let mut effects = DetectedEffects::default();
        effects.io_operations.push("print".to_string());
        effects.global_writes.push("counter".to_string());

        let strings = effects.to_effect_strings();
        assert!(strings.contains(&"io".to_string()));
        assert!(strings.contains(&"global_write".to_string()));
    }

    #[test]
    fn test_analyze_pure_function() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("pure.py");
        std::fs::write(
            &file_path,
            r#"
def add(a, b):
    return a + b

def multiply(x, y):
    return x * y
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        assert_eq!(report.functions.len(), 2);
        let add_fn = report.functions.iter().find(|f| f.name == "add").unwrap();
        assert!(add_fn.classification == "pure", "add function should be pure");
        assert!(add_fn.effects.is_empty());
    }

    #[test]
    fn test_analyze_impure_io() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("impure.py");
        std::fs::write(
            &file_path,
            r#"
def log_message(msg):
    print(msg)
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        assert_eq!(report.functions.len(), 1);
        let log_fn = &report.functions[0];
        assert!(log_fn.classification == "impure", "log_message should be impure");
        assert!(log_fn.effects.iter().any(|e| e.contains("io")));
    }

    #[test]
    fn test_analyze_global_write() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("global.py");
        std::fs::write(
            &file_path,
            r#"
counter = 0

def increment():
    global counter
    counter += 1
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let inc_fn = report
            .functions
            .iter()
            .find(|f| f.name == "increment")
            .unwrap();
        assert!(inc_fn.classification == "impure", "increment should be impure (global write)");
        assert!(inc_fn.effects.iter().any(|e| e.contains("global")));
    }

    #[test]
    fn test_analyze_attribute_write() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("attr.py");
        std::fs::write(
            &file_path,
            r#"
class Counter:
    def __init__(self):
        self.count = 0

    def increment(self):
        self.count += 1
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        // __init__ and increment methods
        assert!(report.functions.len() >= 2);
        let inc_fn = report
            .functions
            .iter()
            .find(|f| f.name == "increment")
            .unwrap();
        assert!(inc_fn.classification == "impure", "increment method should be impure (attribute write)");
    }

    #[test]
    fn test_interprocedural_propagation() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("interproc.py");
        std::fs::write(
            &file_path,
            r#"
def impure_helper():
    print("side effect")

def calls_impure():
    impure_helper()
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let calls_fn = report
            .functions
            .iter()
            .find(|f| f.name == "calls_impure")
            .unwrap();
        assert!(
            calls_fn.classification == "impure",
            "calls_impure should be impure via interprocedural analysis"
        );
    }

    #[test]
    fn test_no_interprocedural_flag() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("interproc.py");
        std::fs::write(
            &file_path,
            r#"
def impure_helper():
    print("side effect")

def calls_impure():
    impure_helper()
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: true, // Disable interprocedural
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        // Without interprocedural, calls_impure has no direct effects
        // It should still be found
        let calls_fn = report.functions.iter().find(|f| f.name == "calls_impure");
        assert!(calls_fn.is_some());
    }

    #[test]
    fn test_confidence_levels() {
        // High confidence - no unknown calls
        let effects = DetectedEffects::default();
        assert_eq!(confidence_from_analysis(&effects), Confidence::High);

        // Medium confidence - few unknown calls
        let mut effects2 = DetectedEffects::default();
        effects2.unknown_calls.push("foo".to_string());
        assert_eq!(confidence_from_analysis(&effects2), Confidence::Medium);

        // Low confidence - many unknown calls
        let mut effects3 = DetectedEffects::default();
        effects3.unknown_calls = vec!["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string()];
        assert_eq!(confidence_from_analysis(&effects3), Confidence::Low);
    }

    #[test]
    fn test_format_purity_text() {
        let report = PurityReport {
            files: vec![FilePurityReport {
                source_file: "test.py".to_string(),
                functions: vec![
                    FunctionPurity {
                        name: "add".to_string(),
                        classification: "pure".to_string(),
                        effects: vec![],
                        confidence: Confidence::High,
                    },
                    FunctionPurity {
                        name: "log".to_string(),
                        classification: "impure".to_string(),
                        effects: vec!["io".to_string()],
                        confidence: Confidence::High,
                    },
                ],
                pure_count: 1,
            }],
            total_functions: 2,
            total_pure: 1,
        };

        let text = format_purity_text(&report);
        assert!(text.contains("Function: add"));
        assert!(text.contains("Classification: pure"));
        assert!(text.contains("Function: log"));
        assert!(text.contains("Classification: impure"));
        assert!(text.contains("Total: 2 functions, 1 pure"));
    }

    #[test]
    fn test_file_not_found() {
        let args = PurityArgs {
            path: PathBuf::from("/nonexistent/file.py"),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let result = analyze_purity_file(Path::new("/nonexistent/file.py"), &args);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_file() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("empty.py");
        std::fs::write(&file_path, "# Empty file\n").unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();
        assert!(report.functions.is_empty());
        assert_eq!(report.pure_count, 0);
    }

    #[test]
    fn test_collection_mutation_detection() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("collection.py");
        std::fs::write(
            &file_path,
            r#"
def mutate_list(items):
    items.append("new")
    return items
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let mutate_fn = report
            .functions
            .iter()
            .find(|f| f.name == "mutate_list")
            .unwrap();
        assert!(mutate_fn.classification == "impure", "mutate_list should be impure (collection mutation)");
    }

    #[test]
    fn test_analyze_go_function() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("hello.go");
        std::fs::write(
            &file_path,
            r#"package main

func Add(a int, b int) int {
	return a + b
}

func PrintHello() {
	fmt.Println("hello")
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        // Should find both Go functions
        assert!(
            report.functions.len() >= 2,
            "Expected at least 2 Go functions, found {}",
            report.functions.len()
        );

        let add_fn = report.functions.iter().find(|f| f.name == "Add");
        assert!(add_fn.is_some(), "Should find Go function 'Add'");
    }

    #[test]
    fn test_analyze_go_method() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("repo.go");
        std::fs::write(
            &file_path,
            r#"package main

type Repo struct {
	count int
}

func (r *Repo) Increment() {
	r.count++
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: Some("Increment".to_string()),
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        assert_eq!(report.functions.len(), 1, "Should find Go method 'Increment'");
        assert_eq!(report.functions[0].name, "Increment");
    }

    // =========================================================================
    // Per-language purity detection tests (P0.5 fix)
    // =========================================================================

    #[test]
    fn test_go_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("hello.go");
        std::fs::write(
            &file_path,
            r#"package main

func Add(a int, b int) int {
	return a + b
}

func PrintHello() {
	fmt.Println("hello")
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "Add").unwrap();
        assert_eq!(add_fn.classification, "pure", "Go Add should be pure");

        let print_fn = report.functions.iter().find(|f| f.name == "PrintHello").unwrap();
        assert_eq!(
            print_fn.classification, "impure",
            "Go PrintHello should be impure (calls fmt.Println)"
        );
    }

    #[test]
    fn test_go_impure_calls_detected() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("rng.go");
        std::fs::write(
            &file_path,
            r#"package main

func GetRandom() int {
	return rand.Int()
}

func GetTime() int {
	return time.Now()
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let rng_fn = report.functions.iter().find(|f| f.name == "GetRandom").unwrap();
        assert_eq!(
            rng_fn.classification, "impure",
            "Go GetRandom should be impure (calls rand.Int)"
        );

        let time_fn = report.functions.iter().find(|f| f.name == "GetTime").unwrap();
        assert_eq!(
            time_fn.classification, "impure",
            "Go GetTime should be impure (calls time.Now)"
        );
    }

    #[test]
    fn test_js_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("app.js");
        std::fs::write(
            &file_path,
            r#"
function add(a, b) {
    return a + b;
}

function logMessage(msg) {
    console.log(msg);
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "add").unwrap();
        assert_eq!(add_fn.classification, "pure", "JS add should be pure");

        let log_fn = report.functions.iter().find(|f| f.name == "logMessage").unwrap();
        assert_eq!(
            log_fn.classification, "impure",
            "JS logMessage should be impure (calls console.log)"
        );
    }

    #[test]
    fn test_js_impure_calls_detected() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("timer.js");
        std::fs::write(
            &file_path,
            r#"
function getRandom() {
    return Math.random();
}

function later(fn) {
    setTimeout(fn, 100);
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let rng_fn = report.functions.iter().find(|f| f.name == "getRandom").unwrap();
        assert_eq!(
            rng_fn.classification, "impure",
            "JS getRandom should be impure (calls Math.random)"
        );
    }

    #[test]
    fn test_js_collection_mutations_detected() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("arr.js");
        std::fs::write(
            &file_path,
            r#"
function addItem(arr, item) {
    arr.push(item);
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "addItem").unwrap();
        assert_eq!(
            add_fn.classification, "impure",
            "JS addItem should be impure (calls arr.push)"
        );
    }

    #[test]
    fn test_rust_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("lib.rs");
        std::fs::write(
            &file_path,
            r#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn greet(name: &str) {
    println!("Hello, {}", name);
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "add").unwrap();
        assert_eq!(add_fn.classification, "pure", "Rust add should be pure");

        let greet_fn = report.functions.iter().find(|f| f.name == "greet").unwrap();
        assert_eq!(
            greet_fn.classification, "impure",
            "Rust greet should be impure (calls println!)"
        );
    }

    #[test]
    fn test_java_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("Main.java");
        std::fs::write(
            &file_path,
            r#"
class Main {
    int add(int a, int b) {
        return a + b;
    }

    void greet(String name) {
        System.out.println("Hello " + name);
    }
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "add").unwrap();
        assert_eq!(add_fn.classification, "pure", "Java add should be pure");

        let greet_fn = report.functions.iter().find(|f| f.name == "greet").unwrap();
        assert_eq!(
            greet_fn.classification, "impure",
            "Java greet should be impure (calls System.out.println)"
        );
    }

    #[test]
    fn test_c_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("main.c");
        std::fs::write(
            &file_path,
            r#"
int add(int a, int b) {
    return a + b;
}

void greet(const char* name) {
    printf("Hello %s\n", name);
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "add").unwrap();
        assert_eq!(add_fn.classification, "pure", "C add should be pure");

        let greet_fn = report.functions.iter().find(|f| f.name == "greet").unwrap();
        assert_eq!(
            greet_fn.classification, "impure",
            "C greet should be impure (calls printf)"
        );
    }

    #[test]
    fn test_elixir_call_name_extraction() {
        // Test that extract_call_name works correctly for Elixir IO.puts call
        let src = b"IO.puts(x)";
        let ts_lang = ParserPool::get_ts_language(Language::Elixir).unwrap();
        let mut parser = Parser::new();
        parser.set_language(&ts_lang).unwrap();
        // Wrap in a defmodule/def to get proper parsing context
        let full_src = b"defmodule T do\n  def f do\n    IO.puts(x)\n  end\nend";
        let tree = parser.parse(full_src.as_ref(), None).unwrap();

        // Find the IO.puts call node by traversing
        fn find_io_puts_call<'a>(node: Node<'a>, source: &[u8]) -> Option<Node<'a>> {
            if node.kind() == "call" {
                let text = node.utf8_text(source).unwrap_or("");
                if text.starts_with("IO.puts") {
                    return Some(node);
                }
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(found) = find_io_puts_call(child, source) {
                    return Some(found);
                }
            }
            None
        }

        let io_puts_call = find_io_puts_call(tree.root_node(), full_src.as_ref())
            .expect("Should find IO.puts call node in Elixir AST");
        let name = extract_call_name(io_puts_call, full_src.as_ref());
        assert!(
            name.as_deref() == Some("IO.puts"),
            "Elixir IO.puts call name should be 'IO.puts', got: {:?}",
            name
        );
    }

    #[test]
    fn test_elixir_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("test.ex");
        std::fs::write(
            &file_path,
            r#"defmodule Test do
  def pure_add(a, b) do
    a + b
  end

  def impure_print(x) do
    IO.puts(x)
    x
  end
end
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "pure_add").unwrap();
        assert_eq!(add_fn.classification, "pure", "Elixir pure_add should be pure");

        let print_fn = report.functions.iter().find(|f| f.name == "impure_print").unwrap();
        assert_eq!(
            print_fn.classification, "impure",
            "Elixir impure_print should be impure (calls IO.puts)"
        );
    }

    #[test]
    fn test_csharp_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("Test.cs");
        std::fs::write(
            &file_path,
            r#"
using System;

class Test {
    static int pure_add(int a, int b) {
        return a + b;
    }

    static int impure_print(int x) {
        Console.WriteLine(x);
        return x;
    }
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "pure_add").unwrap();
        assert_eq!(add_fn.classification, "pure", "C# pure_add should be pure");

        let print_fn = report.functions.iter().find(|f| f.name == "impure_print").unwrap();
        assert_eq!(
            print_fn.classification, "impure",
            "C# impure_print should be impure (calls Console.WriteLine)"
        );
    }

    #[test]
    fn test_php_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("test.php");
        std::fs::write(
            &file_path,
            "<?php\nfunction pure_add($a, $b) {\n    return $a + $b;\n}\n\nfunction impure_print($x) {\n    echo $x;\n    return $x;\n}\n\nfunction impure_random() {\n    return rand();\n}\n?>",
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "pure_add").unwrap();
        assert_eq!(add_fn.classification, "pure", "PHP pure_add should be pure");

        let print_fn = report.functions.iter().find(|f| f.name == "impure_print").unwrap();
        assert_eq!(
            print_fn.classification, "impure",
            "PHP impure_print should be impure (calls echo)"
        );

        let rand_fn = report.functions.iter().find(|f| f.name == "impure_random").unwrap();
        assert_eq!(
            rand_fn.classification, "impure",
            "PHP impure_random should be impure (calls rand)"
        );
    }

    #[test]
    fn test_lua_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("test.lua");
        std::fs::write(
            &file_path,
            r#"
function pure_add(a, b)
    return a + b
end

function impure_print(x)
    print(x)
    return x
end

function impure_random()
    return math.random()
end
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "pure_add").unwrap();
        assert_eq!(add_fn.classification, "pure", "Lua pure_add should be pure");

        let print_fn = report.functions.iter().find(|f| f.name == "impure_print").unwrap();
        assert_eq!(
            print_fn.classification, "impure",
            "Lua impure_print should be impure (calls print)"
        );

        let rand_fn = report.functions.iter().find(|f| f.name == "impure_random").unwrap();
        assert_eq!(
            rand_fn.classification, "impure",
            "Lua impure_random should be impure (calls math.random)"
        );
    }

    #[test]
    fn test_ruby_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("test.rb");
        std::fs::write(
            &file_path,
            r#"
def pure_add(a, b)
  a + b
end

def impure_print(x)
  puts x
  x
end

def impure_random
  rand
end
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "pure_add").unwrap();
        assert_eq!(add_fn.classification, "pure", "Ruby pure_add should be pure");

        let print_fn = report.functions.iter().find(|f| f.name == "impure_print").unwrap();
        assert_eq!(
            print_fn.classification, "impure",
            "Ruby impure_print should be impure (calls puts)"
        );

        let rand_fn = report.functions.iter().find(|f| f.name == "impure_random").unwrap();
        assert_eq!(
            rand_fn.classification, "impure",
            "Ruby impure_random should be impure (calls rand)"
        );
    }

    #[test]
    fn test_swift_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("test.swift");
        std::fs::write(
            &file_path,
            r#"
func pure_add(a: Int, b: Int) -> Int {
    return a + b
}

func impure_print(x: Int) -> Int {
    print(x)
    return x
}

func impure_random() -> UInt32 {
    return arc4random()
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "pure_add").unwrap();
        assert_eq!(add_fn.classification, "pure", "Swift pure_add should be pure");

        let print_fn = report.functions.iter().find(|f| f.name == "impure_print").unwrap();
        assert_eq!(
            print_fn.classification, "impure",
            "Swift impure_print should be impure (calls print)"
        );

        let rand_fn = report.functions.iter().find(|f| f.name == "impure_random").unwrap();
        assert_eq!(
            rand_fn.classification, "impure",
            "Swift impure_random should be impure (calls arc4random)"
        );
    }

    #[test]
    fn test_scala_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("Test.scala");
        std::fs::write(
            &file_path,
            r#"
object Test {
  def pure_add(a: Int, b: Int): Int = {
    a + b
  }

  def impure_print(x: Int): Int = {
    println(x)
    x
  }
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "pure_add").unwrap();
        assert_eq!(add_fn.classification, "pure", "Scala pure_add should be pure");

        let print_fn = report.functions.iter().find(|f| f.name == "impure_print").unwrap();
        assert_eq!(
            print_fn.classification, "impure",
            "Scala impure_print should be impure (calls println)"
        );
    }

    #[test]
    fn test_kotlin_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("test.kt");
        std::fs::write(
            &file_path,
            r#"
fun pure_add(a: Int, b: Int): Int {
    return a + b
}

fun impure_print(x: Int): Int {
    println(x)
    return x
}
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "pure_add").unwrap();
        assert_eq!(add_fn.classification, "pure", "Kotlin pure_add should be pure");

        let print_fn = report.functions.iter().find(|f| f.name == "impure_print").unwrap();
        assert_eq!(
            print_fn.classification, "impure",
            "Kotlin impure_print should be impure (calls println)"
        );
    }

    #[test]
    fn test_ocaml_io_detected_as_impure() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("test.ml");
        std::fs::write(
            &file_path,
            r#"
let pure_add a b =
  a + b

let impure_print x =
  print_endline (string_of_int x);
  x

let impure_random () =
  Random.int 100
"#,
        )
        .unwrap();

        let args = PurityArgs {
            path: file_path.clone(),
            function: None,
            no_interprocedural: false,
            include_tests: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_purity_file(&file_path, &args).unwrap();

        let add_fn = report.functions.iter().find(|f| f.name == "pure_add").unwrap();
        assert_eq!(add_fn.classification, "pure", "OCaml pure_add should be pure");

        let print_fn = report.functions.iter().find(|f| f.name == "impure_print").unwrap();
        assert_eq!(
            print_fn.classification, "impure",
            "OCaml impure_print should be impure (calls print_endline)"
        );

        let rand_fn = report.functions.iter().find(|f| f.name == "impure_random").unwrap();
        assert_eq!(
            rand_fn.classification, "impure",
            "OCaml impure_random should be impure (calls Random.int)"
        );
    }

    #[test]
    fn test_dispatch_functions_cover_all_languages() {
        // Verify the dispatch functions exist and return non-empty arrays
        // for a sample of languages
        let languages = vec![
            Language::Python,
            Language::JavaScript,
            Language::TypeScript,
            Language::Go,
            Language::Rust,
            Language::Java,
            Language::C,
            Language::Cpp,
            Language::Ruby,
            Language::Php,
            Language::Kotlin,
            Language::Swift,
            Language::CSharp,
            Language::Scala,
            Language::Elixir,
            Language::Lua,
            Language::Luau,
            Language::Ocaml,
        ];

        for lang in languages {
            let io_ops = io_ops_for_language(lang);
            assert!(!io_ops.is_empty(), "io_ops_for_language({:?}) should not be empty", lang);

            let impure = impure_calls_for_language(lang);
            assert!(!impure.is_empty(), "impure_calls_for_language({:?}) should not be empty", lang);

            let mutations = collection_mutations_for_language(lang);
            assert!(!mutations.is_empty(), "collection_mutations_for_language({:?}) should not be empty", lang);
        }
    }

    #[test]
    fn test_json_output_escapes_control_chars() {
        // Ensure that source snippets with tabs/control chars produce valid JSON
        let report = PurityReport {
            files: vec![FilePurityReport {
                source_file: "test.py".to_string(),
                functions: vec![
                    FunctionPurity {
                        name: "tabbed\tfunc".to_string(),  // tab in name
                        classification: "pure".to_string(),
                        effects: vec![],
                        confidence: Confidence::High,
                    },
                ],
                pure_count: 1,
            }],
            total_functions: 1,
            total_pure: 1,
        };

        let json = serde_json::to_string_pretty(&report).unwrap();
        // The tab should be escaped as \t in JSON
        assert!(!json.contains('\t'), "JSON output should not contain raw tab characters");
        // Verify it's valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.is_object());
    }
}
