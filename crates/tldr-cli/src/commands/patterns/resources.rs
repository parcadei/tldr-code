//! Resources Command - Resource Lifecycle Analysis
//!
//! Analyzes resource lifecycle to detect leaks, double-close, and use-after-close issues.
//!
//! # Analysis Types
//!
//! - R1: Resource detection - Identify resources requiring close
//! - R2: Close verification - All-paths leak detection
//! - R3: Double-close detection - Closing resources twice
//! - R4: Use-after-close - Using closed resources
//! - R6: Context manager suggestions - Suggest `with` statement
//! - R7: Leak path enumeration - Detailed paths to leaks
//! - R9: Constraint generation - LLM-ready constraints
//!
//! # TIGER Mitigations
//!
//! - T04: MAX_PATHS=1000 with early termination for path enumeration
//!
//! # Example
//!
//! ```bash
//! # Analyze a single file
//! tldr resources src/db.py
//!
//! # Analyze specific function
//! tldr resources src/db.py query
//!
//! # Check all issues
//! tldr resources src/db.py --check-all
//!
//! # Show leak paths
//! tldr resources src/db.py --show-paths
//! ```

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use clap::Args;
use tree_sitter::{Node, Parser};

use tldr_core::ast::ParserPool;
use tldr_core::types::Language;

use super::error::{PatternsError, PatternsResult};
use super::types::{
    ContextSuggestion, DoubleCloseInfo, LeakInfo, OutputFormat, ResourceConstraint, ResourceInfo,
    ResourceReport, ResourceSummary, UseAfterCloseInfo,
};
use super::validation::{read_file_safe, validate_file_path, validate_file_path_in_project};
use crate::output::OutputFormat as GlobalOutputFormat;

// =============================================================================
// TIGER-04: Path Enumeration Limit
// =============================================================================

/// Maximum paths to enumerate before early termination (TIGER-04).
pub const MAX_PATHS: usize = 1000;

// =============================================================================
// Resource Detection Constants (Multi-Language)
// =============================================================================

/// Resource pattern for a specific language: (creator_function, resource_type, closer_functions)
struct LangResourcePatterns {
    /// Functions that create resources requiring cleanup
    creators: &'static [(&'static str, &'static str)], // (func_name, resource_type)
    /// Methods/functions that release resources
    closers: &'static [&'static str],
    /// Function node kinds for this language in tree-sitter
    function_kinds: &'static [&'static str],
    /// Name field for function nodes (usually "name")
    name_field: &'static str,
    /// Body node kind ("block" for Python, "statement_block" for TS, etc.)
    body_kinds: &'static [&'static str],
    /// Assignment node kinds
    assignment_kinds: &'static [&'static str],
    /// Return statement kinds
    return_kinds: &'static [&'static str],
    /// If statement kinds
    if_kinds: &'static [&'static str],
    /// Loop statement kinds
    loop_kinds: &'static [&'static str],
    /// Try statement kinds
    try_kinds: &'static [&'static str],
    /// Context manager / RAII / defer kinds
    cleanup_block_kinds: &'static [&'static str],
}

fn get_resource_patterns(lang: Language) -> LangResourcePatterns {
    match lang {
        Language::Python => LangResourcePatterns {
            creators: &[
                ("open", "file"),
                ("socket", "socket"),
                ("create_connection", "socket"),
                ("connect", "connection"),
                ("cursor", "cursor"),
                ("urlopen", "url_connection"),
                ("request", "http_connection"),
                ("popen", "process"),
                ("Popen", "process"),
                ("Lock", "lock"),
                ("RLock", "lock"),
                ("Semaphore", "semaphore"),
                ("Event", "event"),
                ("Condition", "condition"),
            ],
            closers: &[
                "close",
                "shutdown",
                "disconnect",
                "release",
                "dispose",
                "cleanup",
                "terminate",
                "__exit__",
            ],
            function_kinds: &["function_definition"],
            name_field: "name",
            body_kinds: &["block"],
            assignment_kinds: &["assignment"],
            return_kinds: &["return_statement", "raise_statement"],
            if_kinds: &["if_statement"],
            loop_kinds: &["for_statement", "while_statement"],
            try_kinds: &["try_statement"],
            cleanup_block_kinds: &["with_statement"],
        },
        Language::Go => LangResourcePatterns {
            creators: &[
                ("Open", "file"),
                ("Create", "file"),
                ("OpenFile", "file"),
                ("NewFile", "file"),
                ("Dial", "connection"),
                ("DialTCP", "connection"),
                ("DialUDP", "connection"),
                ("DialTimeout", "connection"),
                ("Listen", "listener"),
                ("ListenTCP", "listener"),
                ("ListenAndServe", "server"),
                ("NewReader", "reader"),
                ("NewWriter", "writer"),
                ("NewScanner", "scanner"),
                ("Get", "http_response"),
                ("Post", "http_response"),
                ("NewRequest", "http_request"),
                ("Connect", "connection"),
                ("NewClient", "client"),
                ("Pipe", "pipe"),
                ("TempFile", "file"),
            ],
            closers: &["Close", "Shutdown", "Stop", "Release", "Flush"],
            function_kinds: &["function_declaration", "method_declaration"],
            name_field: "name",
            body_kinds: &["block"],
            assignment_kinds: &["short_var_declaration", "assignment_statement"],
            return_kinds: &["return_statement"],
            if_kinds: &["if_statement"],
            loop_kinds: &["for_statement"],
            try_kinds: &[],
            cleanup_block_kinds: &["defer_statement"],
        },
        Language::Rust => LangResourcePatterns {
            creators: &[
                ("open", "file"),
                ("create", "file"),
                ("connect", "connection"),
                ("bind", "listener"),
                ("lock", "mutex_guard"),
                ("read_lock", "rwlock_guard"),
                ("write_lock", "rwlock_guard"),
                ("try_lock", "mutex_guard"),
                ("spawn", "thread_handle"),
                ("new", "resource"),
                ("from_raw_fd", "file_descriptor"),
                ("into_raw_fd", "file_descriptor"),
                ("TcpStream", "connection"),
                ("TcpListener", "listener"),
                ("UdpSocket", "socket"),
                ("File", "file"),
                ("BufReader", "reader"),
                ("BufWriter", "writer"),
            ],
            closers: &["drop", "close", "shutdown", "flush", "sync_all"],
            function_kinds: &["function_item"],
            name_field: "name",
            body_kinds: &["block"],
            assignment_kinds: &["let_declaration"],
            return_kinds: &["return_expression"],
            if_kinds: &["if_expression"],
            loop_kinds: &["for_expression", "while_expression", "loop_expression"],
            try_kinds: &[],
            cleanup_block_kinds: &[],
        },
        Language::Java => LangResourcePatterns {
            creators: &[
                ("FileInputStream", "file_stream"),
                ("FileOutputStream", "file_stream"),
                ("FileReader", "reader"),
                ("FileWriter", "writer"),
                ("BufferedReader", "reader"),
                ("BufferedWriter", "writer"),
                ("InputStreamReader", "reader"),
                ("OutputStreamWriter", "writer"),
                ("PrintWriter", "writer"),
                ("Scanner", "scanner"),
                ("Socket", "socket"),
                ("ServerSocket", "server_socket"),
                ("Connection", "connection"),
                ("getConnection", "connection"),
                ("prepareStatement", "statement"),
                ("createStatement", "statement"),
                ("openConnection", "connection"),
                ("newInputStream", "stream"),
                ("newOutputStream", "stream"),
                ("RandomAccessFile", "file"),
            ],
            closers: &[
                "close",
                "shutdown",
                "disconnect",
                "dispose",
                "release",
                "flush",
            ],
            function_kinds: &["method_declaration", "constructor_declaration"],
            name_field: "name",
            body_kinds: &["block"],
            assignment_kinds: &["local_variable_declaration"],
            return_kinds: &["return_statement", "throw_statement"],
            if_kinds: &["if_statement"],
            loop_kinds: &[
                "for_statement",
                "enhanced_for_statement",
                "while_statement",
                "do_statement",
            ],
            try_kinds: &["try_statement", "try_with_resources_statement"],
            cleanup_block_kinds: &["try_with_resources_statement"],
        },
        Language::TypeScript | Language::JavaScript => LangResourcePatterns {
            creators: &[
                ("open", "file"),
                ("openSync", "file"),
                ("createReadStream", "stream"),
                ("createWriteStream", "stream"),
                ("createServer", "server"),
                ("connect", "connection"),
                ("createConnection", "connection"),
                ("fetch", "response"),
                ("request", "request"),
                ("get", "request"),
                ("post", "request"),
                ("WebSocket", "websocket"),
                ("createPool", "pool"),
                ("getConnection", "connection"),
            ],
            closers: &[
                "close",
                "end",
                "destroy",
                "disconnect",
                "release",
                "abort",
                "unref",
            ],
            function_kinds: &[
                "function_declaration",
                "arrow_function",
                "method_definition",
                "function",
            ],
            name_field: "name",
            body_kinds: &["statement_block"],
            assignment_kinds: &[
                "variable_declaration",
                "lexical_declaration",
                "assignment_expression",
            ],
            return_kinds: &["return_statement", "throw_statement"],
            if_kinds: &["if_statement"],
            loop_kinds: &[
                "for_statement",
                "for_in_statement",
                "while_statement",
                "do_statement",
            ],
            try_kinds: &["try_statement"],
            cleanup_block_kinds: &[],
        },
        Language::C => LangResourcePatterns {
            creators: &[
                ("fopen", "file"),
                ("fdopen", "file"),
                ("tmpfile", "file"),
                ("open", "file_descriptor"),
                ("creat", "file_descriptor"),
                ("socket", "socket"),
                ("accept", "socket"),
                ("malloc", "memory"),
                ("calloc", "memory"),
                ("realloc", "memory"),
                ("strdup", "memory"),
                ("mmap", "memory_map"),
                ("opendir", "directory"),
                ("popen", "process"),
                ("dlopen", "dynamic_lib"),
                ("CreateFile", "file_handle"),
            ],
            closers: &[
                "fclose",
                "close",
                "free",
                "munmap",
                "closedir",
                "pclose",
                "dlclose",
                "shutdown",
                "CloseHandle",
            ],
            function_kinds: &["function_definition"],
            name_field: "declarator",
            body_kinds: &["compound_statement"],
            assignment_kinds: &["declaration", "assignment_expression"],
            return_kinds: &["return_statement"],
            if_kinds: &["if_statement"],
            loop_kinds: &["for_statement", "while_statement", "do_statement"],
            try_kinds: &[],
            cleanup_block_kinds: &[],
        },
        Language::Cpp => LangResourcePatterns {
            creators: &[
                ("fopen", "file"),
                ("open", "file_descriptor"),
                ("socket", "socket"),
                ("malloc", "memory"),
                ("calloc", "memory"),
                ("realloc", "memory"),
                ("new", "heap_object"),
                ("make_unique", "unique_ptr"),
                ("make_shared", "shared_ptr"),
                ("ifstream", "file_stream"),
                ("ofstream", "file_stream"),
                ("fstream", "file_stream"),
                ("CreateFile", "file_handle"),
                ("connect", "connection"),
            ],
            closers: &[
                "fclose",
                "close",
                "free",
                "delete",
                "shutdown",
                "release",
                "CloseHandle",
                "destroy",
            ],
            function_kinds: &["function_definition"],
            name_field: "declarator",
            body_kinds: &["compound_statement"],
            assignment_kinds: &["declaration", "assignment_expression"],
            return_kinds: &["return_statement", "throw_statement"],
            if_kinds: &["if_statement"],
            loop_kinds: &[
                "for_statement",
                "while_statement",
                "do_statement",
                "for_range_loop",
            ],
            try_kinds: &["try_statement"],
            cleanup_block_kinds: &[],
        },
        Language::Ruby => LangResourcePatterns {
            creators: &[
                ("open", "file"),
                ("new", "resource"),
                ("popen", "process"),
                ("TCPSocket", "socket"),
                ("UNIXSocket", "socket"),
                ("connect", "connection"),
            ],
            closers: &["close", "shutdown", "disconnect", "release"],
            function_kinds: &["method", "singleton_method"],
            name_field: "name",
            body_kinds: &["body_statement"],
            assignment_kinds: &["assignment"],
            return_kinds: &["return", "raise"],
            if_kinds: &["if", "unless"],
            loop_kinds: &["for", "while", "until"],
            try_kinds: &["begin"],
            cleanup_block_kinds: &["do_block"],
        },
        Language::CSharp => LangResourcePatterns {
            creators: &[
                ("FileStream", "file_stream"),
                ("StreamReader", "reader"),
                ("StreamWriter", "writer"),
                ("File.Open", "file"),
                ("File.OpenRead", "file"),
                ("File.OpenWrite", "file"),
                ("SqlConnection", "connection"),
                ("HttpClient", "http_client"),
                ("TcpClient", "tcp_client"),
                ("Socket", "socket"),
            ],
            closers: &["Close", "Dispose", "Shutdown", "Release", "Flush"],
            function_kinds: &["method_declaration", "constructor_declaration"],
            name_field: "name",
            body_kinds: &["block"],
            assignment_kinds: &["local_declaration_statement", "assignment_expression"],
            return_kinds: &["return_statement", "throw_statement"],
            if_kinds: &["if_statement"],
            loop_kinds: &[
                "for_statement",
                "foreach_statement",
                "while_statement",
                "do_statement",
            ],
            try_kinds: &["try_statement"],
            cleanup_block_kinds: &["using_statement"],
        },
        Language::Php => LangResourcePatterns {
            creators: &[
                ("fopen", "file"),
                ("tmpfile", "file"),
                ("fsockopen", "socket"),
                ("pfsockopen", "socket"),
                ("curl_init", "curl"),
                ("mysqli_connect", "connection"),
                ("PDO", "connection"),
                ("popen", "process"),
                ("opendir", "directory"),
            ],
            closers: &[
                "fclose",
                "curl_close",
                "mysqli_close",
                "pclose",
                "closedir",
                "close",
            ],
            function_kinds: &["function_definition", "method_declaration"],
            name_field: "name",
            body_kinds: &["compound_statement"],
            assignment_kinds: &["assignment_expression"],
            return_kinds: &["return_statement", "throw_expression"],
            if_kinds: &["if_statement"],
            loop_kinds: &[
                "for_statement",
                "foreach_statement",
                "while_statement",
                "do_statement",
            ],
            try_kinds: &["try_statement"],
            cleanup_block_kinds: &[],
        },
        Language::Elixir => LangResourcePatterns {
            creators: &[
                ("open", "file"),
                ("open!", "file"),
                ("connect", "connection"),
                ("start_link", "process"),
                ("start", "process"),
            ],
            closers: &["close", "stop", "disconnect"],
            function_kinds: &["call"], // Elixir uses `def` as a macro call
            name_field: "target",
            body_kinds: &["do_block"],
            assignment_kinds: &["binary_operator"], // = operator
            return_kinds: &[],
            if_kinds: &["call"],   // if is a macro
            loop_kinds: &["call"], // for/Enum.each are calls
            try_kinds: &["call"],  // try is a macro
            cleanup_block_kinds: &[],
        },
        Language::Scala => LangResourcePatterns {
            creators: &[
                ("Source", "source"),
                ("fromFile", "source"),
                ("FileInputStream", "stream"),
                ("FileOutputStream", "stream"),
                ("BufferedSource", "source"),
                ("getConnection", "connection"),
            ],
            closers: &["close", "shutdown", "disconnect", "dispose"],
            function_kinds: &["function_definition"],
            name_field: "name",
            body_kinds: &["block"],
            assignment_kinds: &["val_definition", "var_definition"],
            return_kinds: &["return_expression"],
            if_kinds: &["if_expression"],
            loop_kinds: &["for_expression", "while_expression"],
            try_kinds: &["try_expression"],
            cleanup_block_kinds: &[],
        },
        Language::Kotlin => LangResourcePatterns {
            creators: &[
                ("FileInputStream", "file_stream"),
                ("FileOutputStream", "file_stream"),
                ("FileReader", "reader"),
                ("FileWriter", "writer"),
                ("BufferedReader", "reader"),
                ("BufferedWriter", "writer"),
                ("InputStreamReader", "reader"),
                ("OutputStreamWriter", "writer"),
                ("PrintWriter", "writer"),
                ("Scanner", "scanner"),
                ("Socket", "socket"),
                ("ServerSocket", "server_socket"),
                ("getConnection", "connection"),
                ("openConnection", "connection"),
                ("File", "file"),
                ("RandomAccessFile", "file"),
            ],
            closers: &["close", "shutdown", "dispose", "use"],
            function_kinds: &["function_declaration"],
            name_field: "name",
            body_kinds: &["function_body"],
            assignment_kinds: &["property_declaration", "assignment"],
            return_kinds: &["jump_expression"],
            if_kinds: &["if_expression"],
            loop_kinds: &["for_statement", "while_statement"],
            try_kinds: &["try_expression"],
            cleanup_block_kinds: &["call_expression"], // .use { } block
        },
        Language::Swift => LangResourcePatterns {
            creators: &[
                ("FileHandle", "file_handle"),
                ("OutputStream", "stream"),
                ("InputStream", "stream"),
                ("URLSession", "session"),
                ("FileManager", "file_manager"),
                ("fopen", "file"),
                ("open", "file"),
                ("Socket", "socket"),
                ("NWConnection", "connection"),
            ],
            closers: &[
                "closeFile",
                "close",
                "shutdown",
                "invalidateAndCancel",
                "cancel",
            ],
            function_kinds: &["function_declaration"],
            name_field: "name",
            body_kinds: &["function_body"],
            assignment_kinds: &["property_declaration", "directly_assignable_expression"],
            return_kinds: &["control_transfer_statement"],
            if_kinds: &["if_statement"],
            loop_kinds: &["for_statement", "while_statement"],
            try_kinds: &["do_statement"], // do { } catch { }
            cleanup_block_kinds: &[],     // defer detected differently
        },
        Language::Ocaml => LangResourcePatterns {
            creators: &[
                ("open_in", "input_channel"),
                ("open_out", "output_channel"),
                ("open_in_bin", "input_channel"),
                ("open_out_bin", "output_channel"),
                ("Unix.openfile", "file_descriptor"),
                ("Unix.socket", "socket"),
                ("open_connection", "connection"),
                ("connect", "connection"),
            ],
            closers: &[
                "close_in",
                "close_out",
                "close_in_noerr",
                "close_out_noerr",
                "Unix.close",
                "close_connection",
            ],
            function_kinds: &["let_binding", "value_definition"],
            name_field: "pattern",
            body_kinds: &["let_expression", "sequence_expression"],
            assignment_kinds: &["let_binding"],
            return_kinds: &[],
            if_kinds: &["if_expression"],
            loop_kinds: &["for_expression", "while_expression"],
            try_kinds: &["try_expression"],
            cleanup_block_kinds: &[],
        },
        Language::Lua | Language::Luau => LangResourcePatterns {
            creators: &[
                ("io.open", "file"),
                ("io.popen", "process"),
                ("io.tmpfile", "file"),
                ("socket.tcp", "socket"),
                ("socket.udp", "socket"),
                ("socket.connect", "connection"),
                ("open", "file"),
            ],
            closers: &["close"],
            function_kinds: &["function_declaration", "function_definition"],
            name_field: "name",
            body_kinds: &["body"],
            assignment_kinds: &["assignment_statement", "variable_declaration"],
            return_kinds: &["return_statement"],
            if_kinds: &["if_statement"],
            loop_kinds: &["for_statement", "for_in_statement", "while_statement"],
            try_kinds: &[],
            cleanup_block_kinds: &[],
        },
    }
}

/// Legacy constant for backward compatibility with tests
pub const RESOURCE_CREATORS: &[&str] = &[
    "open",
    "socket",
    "create_connection",
    "connect",
    "cursor",
    "urlopen",
    "request",
    "popen",
    "Popen",
    "Lock",
    "RLock",
    "Semaphore",
    "Event",
    "Condition",
    "contextlib.closing",
];

/// Legacy constant for backward compatibility with tests
pub const RESOURCE_CLOSERS: &[&str] = &[
    "close",
    "shutdown",
    "disconnect",
    "release",
    "dispose",
    "cleanup",
    "terminate",
    "__exit__",
];

/// Legacy resource type map for backward compatibility with Python detection
const RESOURCE_TYPE_MAP: &[(&str, &str)] = &[
    ("open", "file"),
    ("socket", "socket"),
    ("create_connection", "socket"),
    ("connect", "connection"),
    ("cursor", "cursor"),
    ("urlopen", "url_connection"),
    ("request", "http_connection"),
    ("popen", "process"),
    ("Popen", "process"),
    ("Lock", "lock"),
    ("RLock", "lock"),
    ("Semaphore", "semaphore"),
    ("Event", "event"),
    ("Condition", "condition"),
];

// =============================================================================
// CLI Arguments
// =============================================================================

/// Analyze resource lifecycle to detect leaks, double-close, and use-after-close.
#[derive(Debug, Args, Clone)]
pub struct ResourcesArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function to analyze (optional; analyze all if omitted)
    pub function: Option<String>,

    /// Language filter (auto-detected if omitted)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Run leak detection (R2) - enabled by default
    #[arg(long, default_value = "true")]
    pub check_leaks: bool,

    /// Run double-close detection (R3)
    #[arg(long)]
    pub check_double_close: bool,

    /// Run use-after-close detection (R4)
    #[arg(long)]
    pub check_use_after_close: bool,

    /// Run all checks (R2, R3, R4)
    #[arg(long)]
    pub check_all: bool,

    /// Suggest context manager usage (R6)
    #[arg(long)]
    pub suggest_context: bool,

    /// Show detailed leak paths (R7)
    #[arg(long)]
    pub show_paths: bool,

    /// Generate LLM constraints (R9)
    #[arg(long)]
    pub constraints: bool,

    /// Output summary statistics only
    #[arg(long)]
    pub summary: bool,

    /// Output format (json or text). Prefer global --format/-f flag.
    #[arg(
        long = "output",
        short = 'o',
        hide = true,
        default_value = "json",
        value_enum
    )]
    pub output_format: OutputFormat,

    /// Project root for path validation (optional)
    #[arg(long)]
    pub project_root: Option<PathBuf>,
}

impl ResourcesArgs {
    /// Run the resources analysis command
    pub fn run(&self, global_format: GlobalOutputFormat) -> anyhow::Result<()> {
        run(self.clone(), global_format)
    }
}

// =============================================================================
// Basic Block and Simplified CFG
// =============================================================================

/// A basic block in the simplified control flow graph.
#[derive(Debug, Clone)]
pub struct BasicBlock {
    /// Unique block identifier
    pub id: usize,
    /// Statement nodes in this block (start_byte, end_byte, kind, text)
    pub stmts: Vec<(usize, usize, String, String)>,
    /// Line numbers of statements
    pub lines: Vec<u32>,
    /// Predecessor block IDs
    pub preds: Vec<usize>,
    /// Successor block IDs
    pub succs: Vec<usize>,
    /// Whether this is an entry block
    pub is_entry: bool,
    /// Whether this is an exit block (return/raise/implicit)
    pub is_exit: bool,
    /// Exception handler block IDs (for try blocks)
    pub exception_handlers: Vec<usize>,
}

impl BasicBlock {
    fn new(id: usize) -> Self {
        Self {
            id,
            stmts: Vec::new(),
            lines: Vec::new(),
            preds: Vec::new(),
            succs: Vec::new(),
            is_entry: false,
            is_exit: false,
            exception_handlers: Vec::new(),
        }
    }
}

/// Simplified control flow graph for resource analysis.
#[derive(Debug)]
pub struct SimpleCfg {
    /// Mapping from block ID to BasicBlock
    pub blocks: HashMap<usize, BasicBlock>,
    /// ID of the entry block
    pub entry_block: usize,
    /// IDs of all exit blocks
    pub exit_blocks: Vec<usize>,
    /// Next available block ID
    next_id: usize,
}

impl SimpleCfg {
    fn new() -> Self {
        Self {
            blocks: HashMap::new(),
            entry_block: 0,
            exit_blocks: Vec::new(),
            next_id: 0,
        }
    }

    fn new_block(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.blocks.insert(id, BasicBlock::new(id));
        id
    }

    fn add_edge(&mut self, from: usize, to: usize) {
        if let Some(block) = self.blocks.get_mut(&from) {
            if !block.succs.contains(&to) {
                block.succs.push(to);
            }
        }
        if let Some(block) = self.blocks.get_mut(&to) {
            if !block.preds.contains(&from) {
                block.preds.push(from);
            }
        }
    }

    fn mark_exit(&mut self, id: usize) {
        if let Some(block) = self.blocks.get_mut(&id) {
            block.is_exit = true;
        }
        if !self.exit_blocks.contains(&id) {
            self.exit_blocks.push(id);
        }
    }
}

// =============================================================================
// CFG Builder
// =============================================================================

/// Build a simplified CFG from a function AST.
pub fn build_cfg(func_node: Node, source: &[u8]) -> SimpleCfg {
    let mut cfg = SimpleCfg::new();
    let entry_id = cfg.new_block();
    cfg.entry_block = entry_id;

    if let Some(block) = cfg.blocks.get_mut(&entry_id) {
        block.is_entry = true;
    }

    // Find the function body
    let body = func_node
        .children(&mut func_node.walk())
        .find(|n| n.kind() == "block");

    if let Some(body_node) = body {
        let exit_id = process_statements(&mut cfg, body_node, source, entry_id);
        if let Some(exit) = exit_id {
            // Mark implicit exit if we have a non-exit block at the end
            if !cfg.blocks.get(&exit).is_none_or(|b| b.is_exit) {
                cfg.mark_exit(exit);
            }
        }
    } else {
        // Empty function
        cfg.mark_exit(entry_id);
    }

    cfg
}

fn process_statements(
    cfg: &mut SimpleCfg,
    node: Node,
    source: &[u8],
    mut current: usize,
) -> Option<usize> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Simple statements - add to current block
            "expression_statement"
            | "assignment"
            | "augmented_assignment"
            | "return_statement"
            | "pass_statement"
            | "break_statement"
            | "continue_statement"
            | "raise_statement"
            | "assert_statement"
            | "global_statement"
            | "nonlocal_statement"
            | "import_statement"
            | "import_from_statement"
            | "delete_statement" => {
                let text = node_text(child, source).to_string();
                let line = child.start_position().row as u32 + 1;
                if let Some(block) = cfg.blocks.get_mut(&current) {
                    block.stmts.push((
                        child.start_byte(),
                        child.end_byte(),
                        child.kind().to_string(),
                        text,
                    ));
                    block.lines.push(line);
                }

                // Handle exit statements
                if child.kind() == "return_statement" || child.kind() == "raise_statement" {
                    cfg.mark_exit(current);
                    return None; // No more statements can be executed
                }
            }

            // If statement - creates branches
            "if_statement" => {
                current = process_if_statement(cfg, child, source, current)?;
            }

            // For/while loops
            "for_statement" | "while_statement" => {
                current = process_loop(cfg, child, source, current)?;
            }

            // Try statement
            "try_statement" => {
                current = process_try(cfg, child, source, current)?;
            }

            // With statement (context manager)
            "with_statement" => {
                current = process_with(cfg, child, source, current)?;
            }

            _ => {
                // Unknown or compound statement - add as is
                let text = node_text(child, source).to_string();
                let line = child.start_position().row as u32 + 1;
                if let Some(block) = cfg.blocks.get_mut(&current) {
                    block.stmts.push((
                        child.start_byte(),
                        child.end_byte(),
                        child.kind().to_string(),
                        text,
                    ));
                    block.lines.push(line);
                }
            }
        }
    }

    Some(current)
}

fn process_if_statement(
    cfg: &mut SimpleCfg,
    node: Node,
    source: &[u8],
    current: usize,
) -> Option<usize> {
    // Add the condition to current block
    if let Some(cond) = node.child_by_field_name("condition") {
        let text = node_text(cond, source).to_string();
        let line = cond.start_position().row as u32 + 1;
        if let Some(block) = cfg.blocks.get_mut(&current) {
            block.stmts.push((
                cond.start_byte(),
                cond.end_byte(),
                "condition".to_string(),
                text,
            ));
            block.lines.push(line);
        }
    }

    // Create blocks for true branch
    let true_block = cfg.new_block();
    cfg.add_edge(current, true_block);

    // Find consequence block
    let mut cursor = node.walk();
    let consequence = node.children(&mut cursor).find(|n| n.kind() == "block");
    let true_exit = if let Some(body) = consequence {
        process_statements(cfg, body, source, true_block)
    } else {
        Some(true_block)
    };

    // Find alternative (else/elif)
    let mut cursor = node.walk();
    let alternative = node
        .children(&mut cursor)
        .find(|n| n.kind() == "else_clause" || n.kind() == "elif_clause");

    let false_exit = if let Some(alt) = alternative {
        let false_block = cfg.new_block();
        cfg.add_edge(current, false_block);

        // Find the block within else/elif
        if let Some(alt_body) = alt.children(&mut alt.walk()).find(|n| n.kind() == "block") {
            process_statements(cfg, alt_body, source, false_block)
        } else {
            Some(false_block)
        }
    } else {
        // No else clause - false branch goes to next block
        None
    };

    // Create merge block
    let merge = cfg.new_block();

    if let Some(te) = true_exit {
        cfg.add_edge(te, merge);
    }
    if let Some(fe) = false_exit {
        cfg.add_edge(fe, merge);
    }
    if alternative.is_none() {
        // If no else, false path goes directly from current to merge
        cfg.add_edge(current, merge);
    }

    Some(merge)
}

fn process_loop(cfg: &mut SimpleCfg, node: Node, source: &[u8], current: usize) -> Option<usize> {
    // Create header block
    let header = cfg.new_block();
    cfg.add_edge(current, header);

    // Add loop condition to header
    if let Some(cond) = node.child_by_field_name("condition") {
        let text = node_text(cond, source).to_string();
        let line = cond.start_position().row as u32 + 1;
        if let Some(block) = cfg.blocks.get_mut(&header) {
            block.stmts.push((
                cond.start_byte(),
                cond.end_byte(),
                "loop_condition".to_string(),
                text,
            ));
            block.lines.push(line);
        }
    }

    // Create body block
    let body_block = cfg.new_block();
    cfg.add_edge(header, body_block);

    // Process body
    let body = node
        .children(&mut node.walk())
        .find(|n| n.kind() == "block");
    let body_exit = if let Some(body_node) = body {
        process_statements(cfg, body_node, source, body_block)
    } else {
        Some(body_block)
    };

    // Back edge from body to header
    if let Some(be) = body_exit {
        cfg.add_edge(be, header);
    }

    // Exit block
    let exit = cfg.new_block();
    cfg.add_edge(header, exit); // Loop can exit when condition is false

    Some(exit)
}

fn process_try(cfg: &mut SimpleCfg, node: Node, source: &[u8], current: usize) -> Option<usize> {
    // Create try block
    let try_block = cfg.new_block();
    cfg.add_edge(current, try_block);

    // Find and process try body
    let try_body = node
        .children(&mut node.walk())
        .find(|n| n.kind() == "block");
    let try_exit = if let Some(body) = try_body {
        process_statements(cfg, body, source, try_block)
    } else {
        Some(try_block)
    };

    // Find except handlers
    let mut cursor = node.walk();
    let mut handler_exits = Vec::new();
    for child in node.children(&mut cursor) {
        if child.kind() == "except_clause" {
            let handler_block = cfg.new_block();
            // Exception edge from try block
            cfg.add_edge(try_block, handler_block);
            if let Some(block) = cfg.blocks.get_mut(&try_block) {
                block.exception_handlers.push(handler_block);
            }

            // Process handler body
            if let Some(handler_body) = child
                .children(&mut child.walk())
                .find(|n| n.kind() == "block")
            {
                if let Some(exit) = process_statements(cfg, handler_body, source, handler_block) {
                    handler_exits.push(exit);
                }
            } else {
                handler_exits.push(handler_block);
            }
        }
    }

    // Find finally clause
    let finally_clause = node
        .children(&mut node.walk())
        .find(|n| n.kind() == "finally_clause");

    // Create merge block
    let merge = cfg.new_block();

    if let Some(te) = try_exit {
        if let Some(finally) = finally_clause {
            // Process finally
            let finally_block = cfg.new_block();
            cfg.add_edge(te, finally_block);
            if let Some(finally_body) = finally
                .children(&mut finally.walk())
                .find(|n| n.kind() == "block")
            {
                if let Some(exit) = process_statements(cfg, finally_body, source, finally_block) {
                    cfg.add_edge(exit, merge);
                }
            } else {
                cfg.add_edge(finally_block, merge);
            }
        } else {
            cfg.add_edge(te, merge);
        }
    }

    for he in handler_exits {
        cfg.add_edge(he, merge);
    }

    Some(merge)
}

fn process_with(cfg: &mut SimpleCfg, node: Node, source: &[u8], current: usize) -> Option<usize> {
    // Add with statement to current block (marks context manager entry)
    let text = node_text(node, source).to_string();
    let line = node.start_position().row as u32 + 1;
    if let Some(block) = cfg.blocks.get_mut(&current) {
        block.stmts.push((
            node.start_byte(),
            node.end_byte(),
            "with_statement".to_string(),
            text,
        ));
        block.lines.push(line);
    }

    // Process the with body
    let body = node
        .children(&mut node.walk())
        .find(|n| n.kind() == "block");
    if let Some(body_node) = body {
        process_statements(cfg, body_node, source, current)
    } else {
        Some(current)
    }
}

// =============================================================================
// Resource Detection
// =============================================================================

/// Detected resource information during analysis.
#[derive(Debug, Clone)]
struct DetectedResource {
    /// Variable name holding the resource
    name: String,
    /// Type of resource
    resource_type: String,
    /// Line where resource was created
    line: u32,
    /// Whether it's inside a context manager (with statement)
    in_context_manager: bool,
}

/// Resource detector for finding must-close resources.
pub struct ResourceDetector {
    resources: Vec<DetectedResource>,
    context_manager_vars: HashSet<String>,
    lang: Language,
}

impl ResourceDetector {
    pub fn new() -> Self {
        Self {
            resources: Vec::new(),
            context_manager_vars: HashSet::new(),
            lang: Language::Python,
        }
    }

    pub fn with_language(lang: Language) -> Self {
        Self {
            resources: Vec::new(),
            context_manager_vars: HashSet::new(),
            lang,
        }
    }

    /// Detect resources in a function (legacy Python-only).
    pub fn detect(&mut self, func_node: Node, source: &[u8]) -> Vec<ResourceInfo> {
        self.resources.clear();
        self.context_manager_vars.clear();
        self.visit_node(func_node, source, false);

        self.resources
            .iter()
            .map(|r| ResourceInfo {
                name: r.name.clone(),
                resource_type: r.resource_type.clone(),
                line: r.line,
                closed: r.in_context_manager,
            })
            .collect()
    }

    /// Detect resources using language-specific patterns.
    pub fn detect_with_patterns(&mut self, func_node: Node, source: &[u8]) -> Vec<ResourceInfo> {
        let patterns = get_resource_patterns(self.lang);
        self.resources.clear();
        self.context_manager_vars.clear();
        self.visit_node_multilang(func_node, source, false, &patterns);

        self.resources
            .iter()
            .map(|r| ResourceInfo {
                name: r.name.clone(),
                resource_type: r.resource_type.clone(),
                line: r.line,
                closed: r.in_context_manager,
            })
            .collect()
    }

    fn visit_node(&mut self, node: Node, source: &[u8], in_with: bool) {
        match node.kind() {
            "with_statement" => {
                // Process with_items - they're direct children of with_statement
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "with_item" {
                        self.visit_with_item(child, source);
                    } else if child.kind() == "with_clause" {
                        // Some Python versions use with_clause wrapper
                        let mut inner_cursor = child.walk();
                        for item in child.children(&mut inner_cursor) {
                            if item.kind() == "with_item" {
                                self.visit_with_item(item, source);
                            }
                        }
                    }
                }
                // Recurse into body with context manager flag
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.visit_node(child, source, true);
                }
            }
            "assignment" => {
                self.check_assignment(node, source, in_with);
            }
            _ => {
                // Recurse
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.visit_node(child, source, in_with);
                }
            }
        }
    }

    fn visit_with_item(&mut self, node: Node, source: &[u8]) {
        // with_item structure in tree-sitter-python:
        //   with_item
        //     as_pattern
        //       call (the expression, e.g., open(path))
        //       as_pattern_target
        //         identifier (the variable name, e.g., f)
        //
        // OR (for with expression without 'as'):
        //   with_item
        //     call (the expression only)

        // First check for as_pattern (with ... as var)
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "as_pattern" {
                let mut as_cursor = child.walk();
                let mut call_node: Option<Node> = None;
                let mut target_node: Option<Node> = None;

                for as_child in child.children(&mut as_cursor) {
                    if as_child.kind() == "call" {
                        call_node = Some(as_child);
                    } else if as_child.kind() == "as_pattern_target" {
                        // as_pattern_target contains the identifier
                        if let Some(ident) = as_child.child(0) {
                            if ident.kind() == "identifier" {
                                target_node = Some(ident);
                            }
                        }
                    }
                }

                if let (Some(call), Some(target)) = (call_node, target_node) {
                    let var_name = node_text(target, source).to_string();
                    self.context_manager_vars.insert(var_name.clone());

                    if let Some(resource_type) = self.get_resource_type_from_call(call, source) {
                        self.resources.push(DetectedResource {
                            name: var_name,
                            resource_type,
                            line: node.start_position().row as u32 + 1,
                            in_context_manager: true,
                        });
                    }
                }
            }
        }

        // Also try field names for older tree-sitter versions
        if let Some(target) = node.child_by_field_name("alias") {
            let var_name = node_text(target, source).to_string();
            if !self.context_manager_vars.contains(&var_name) {
                self.context_manager_vars.insert(var_name.clone());

                if let Some(value) = node.child_by_field_name("value") {
                    if let Some(resource_type) = self.get_resource_type_from_call(value, source) {
                        self.resources.push(DetectedResource {
                            name: var_name,
                            resource_type,
                            line: node.start_position().row as u32 + 1,
                            in_context_manager: true,
                        });
                    }
                }
            }
        }
    }

    fn check_assignment(&mut self, node: Node, source: &[u8], in_with: bool) {
        // f = open(...)
        if let Some(left) = node.child_by_field_name("left") {
            if let Some(right) = node.child_by_field_name("right") {
                let var_name = node_text(left, source).to_string();

                if let Some(resource_type) = self.get_resource_type_from_call(right, source) {
                    let in_context = in_with || self.context_manager_vars.contains(&var_name);
                    self.resources.push(DetectedResource {
                        name: var_name,
                        resource_type,
                        line: node.start_position().row as u32 + 1,
                        in_context_manager: in_context,
                    });
                }
            }
        }
    }

    fn get_resource_type_from_call(&self, node: Node, source: &[u8]) -> Option<String> {
        if node.kind() != "call" {
            return None;
        }

        // Get function name
        let func = node.child_by_field_name("function")?;
        let func_text = node_text(func, source);

        // Extract just the function name from attribute access (e.g., "sqlite3.connect" -> "connect")
        let func_name = func_text.split('.').next_back().unwrap_or(func_text);

        // Check if it's a resource creator
        for &creator in RESOURCE_CREATORS {
            if func_name == creator {
                // Find the resource type from the type map
                for &(name, rtype) in RESOURCE_TYPE_MAP {
                    if func_name == name {
                        return Some(rtype.to_string());
                    }
                }
                // Default to the function name as type
                return Some(func_name.to_string());
            }
        }

        None
    }

    // =========================================================================
    // Multi-language methods
    // =========================================================================

    fn visit_node_multilang(
        &mut self,
        node: Node,
        source: &[u8],
        in_cleanup: bool,
        patterns: &LangResourcePatterns,
    ) {
        let kind = node.kind();

        // Check for cleanup block kinds (with, defer, using, try-with-resources)
        if patterns.cleanup_block_kinds.contains(&kind) {
            match self.lang {
                Language::Python => {
                    // Python with_statement: check for with_item children
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        if child.kind() == "with_item" {
                            self.visit_with_item(child, source);
                        } else if child.kind() == "with_clause" {
                            let mut inner_cursor = child.walk();
                            for item in child.children(&mut inner_cursor) {
                                if item.kind() == "with_item" {
                                    self.visit_with_item(item, source);
                                }
                            }
                        }
                    }
                    // Recurse into body with cleanup flag
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        self.visit_node_multilang(child, source, true, patterns);
                    }
                    return;
                }
                Language::Go => {
                    // Go defer: mark any resource in the defer as cleanup-managed
                    // We just recurse with in_cleanup=true
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        self.visit_node_multilang(child, source, true, patterns);
                    }
                    return;
                }
                Language::CSharp => {
                    // C# using statement: resources are auto-disposed
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        self.visit_node_multilang(child, source, true, patterns);
                    }
                    return;
                }
                Language::Java => {
                    // Java try-with-resources: resources in the resource spec are auto-closed
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        self.visit_node_multilang(child, source, true, patterns);
                    }
                    return;
                }
                _ => {}
            }
        }

        // Check for assignment kinds
        if patterns.assignment_kinds.contains(&kind) {
            self.check_assignment_multilang(node, source, in_cleanup, patterns);
        }

        // Recurse
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit_node_multilang(child, source, in_cleanup, patterns);
        }
    }

    fn check_assignment_multilang(
        &mut self,
        node: Node,
        source: &[u8],
        in_cleanup: bool,
        patterns: &LangResourcePatterns,
    ) {
        match self.lang {
            Language::Python => {
                // f = open(...)
                if let Some(left) = node.child_by_field_name("left") {
                    if let Some(right) = node.child_by_field_name("right") {
                        let var_name = node_text(left, source).to_string();
                        if let Some(resource_type) =
                            self.get_resource_type_from_call_multilang(right, source, patterns)
                        {
                            let in_context =
                                in_cleanup || self.context_manager_vars.contains(&var_name);
                            self.resources.push(DetectedResource {
                                name: var_name,
                                resource_type,
                                line: node.start_position().row as u32 + 1,
                                in_context_manager: in_context,
                            });
                        }
                    }
                }
            }
            Language::Go => {
                // Go: f, err := os.Open(...) or f := os.Open(...)
                // short_var_declaration has left and right fields
                // assignment_statement has left and right fields
                if let Some(left) = node.child_by_field_name("left") {
                    if let Some(right) = node.child_by_field_name("right") {
                        // left might be an expression_list with multiple identifiers
                        let var_name = if left.kind() == "expression_list" {
                            // Take first identifier
                            left.child(0).map(|c| node_text(c, source).to_string())
                        } else {
                            Some(node_text(left, source).to_string())
                        };
                        if let Some(var_name) = var_name {
                            if var_name != "_" && var_name != "err" {
                                // Check right side - may be expression_list too
                                let call_node = if right.kind() == "expression_list" {
                                    right.child(0)
                                } else {
                                    Some(right)
                                };
                                if let Some(call_node) = call_node {
                                    if let Some(resource_type) = self
                                        .get_resource_type_from_call_multilang(
                                            call_node, source, patterns,
                                        )
                                    {
                                        self.resources.push(DetectedResource {
                                            name: var_name,
                                            resource_type,
                                            line: node.start_position().row as u32 + 1,
                                            in_context_manager: in_cleanup,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Language::Rust => {
                // let f = File::open(...)?;
                // let_declaration has pattern and value fields
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    if let Some(value) = node.child_by_field_name("value") {
                        let var_name = node_text(pattern, source).to_string();
                        // Rust uses RAII, so most resources are auto-cleaned.
                        // We detect them but mark as closed (RAII)
                        if let Some(resource_type) =
                            self.get_resource_type_from_call_multilang(value, source, patterns)
                        {
                            self.resources.push(DetectedResource {
                                name: var_name,
                                resource_type,
                                line: node.start_position().row as u32 + 1,
                                in_context_manager: true, // RAII: auto-cleaned on drop
                            });
                        }
                    }
                }
            }
            Language::Java | Language::CSharp => {
                // Type var = new Resource(...);
                // local_variable_declaration contains declarator children
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "variable_declarator" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            if let Some(value) = child.child_by_field_name("value") {
                                let var_name = node_text(name_node, source).to_string();
                                if let Some(resource_type) = self
                                    .get_resource_type_from_call_multilang(value, source, patterns)
                                {
                                    self.resources.push(DetectedResource {
                                        name: var_name,
                                        resource_type,
                                        line: node.start_position().row as u32 + 1,
                                        in_context_manager: in_cleanup,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            Language::TypeScript | Language::JavaScript => {
                // const f = fs.open(...); or let f = ...
                // variable_declaration / lexical_declaration contain variable_declarator children
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "variable_declarator" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            if let Some(value) = child.child_by_field_name("value") {
                                let var_name = node_text(name_node, source).to_string();
                                if let Some(resource_type) = self
                                    .get_resource_type_from_call_multilang(value, source, patterns)
                                {
                                    self.resources.push(DetectedResource {
                                        name: var_name,
                                        resource_type,
                                        line: node.start_position().row as u32 + 1,
                                        in_context_manager: in_cleanup,
                                    });
                                }
                            }
                        }
                    }
                }
                // Also handle assignment_expression: f = open(...)
                if node.kind() == "assignment_expression" {
                    if let Some(left) = node.child_by_field_name("left") {
                        if let Some(right) = node.child_by_field_name("right") {
                            let var_name = node_text(left, source).to_string();
                            if let Some(resource_type) =
                                self.get_resource_type_from_call_multilang(right, source, patterns)
                            {
                                self.resources.push(DetectedResource {
                                    name: var_name,
                                    resource_type,
                                    line: node.start_position().row as u32 + 1,
                                    in_context_manager: in_cleanup,
                                });
                            }
                        }
                    }
                }
            }
            Language::C | Language::Cpp => {
                // FILE *f = fopen(...); or void *p = malloc(...);
                // declaration contains init_declarator children
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "init_declarator" {
                        if let Some(declarator) = child.child_by_field_name("declarator") {
                            if let Some(value) = child.child_by_field_name("value") {
                                // declarator might be a pointer_declarator wrapping an identifier
                                let var_name = extract_c_declarator_name(declarator, source);
                                if let Some(var_name) = var_name {
                                    if let Some(resource_type) = self
                                        .get_resource_type_from_call_multilang(
                                            value, source, patterns,
                                        )
                                    {
                                        self.resources.push(DetectedResource {
                                            name: var_name,
                                            resource_type,
                                            line: node.start_position().row as u32 + 1,
                                            in_context_manager: in_cleanup,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                // Also handle assignment_expression
                if node.kind() == "assignment_expression" {
                    if let Some(left) = node.child_by_field_name("left") {
                        if let Some(right) = node.child_by_field_name("right") {
                            let var_name = node_text(left, source).to_string();
                            if let Some(resource_type) =
                                self.get_resource_type_from_call_multilang(right, source, patterns)
                            {
                                self.resources.push(DetectedResource {
                                    name: var_name,
                                    resource_type,
                                    line: node.start_position().row as u32 + 1,
                                    in_context_manager: in_cleanup,
                                });
                            }
                        }
                    }
                }
            }
            Language::Kotlin => {
                // Kotlin: val reader = BufferedReader(FileReader(path))
                // property_declaration has variable_declaration children with name/value
                // or assignment has left/right
                if node.kind() == "property_declaration" {
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        if child.kind() == "variable_declaration" {
                            if let Some(name_node) =
                                child.child_by_field_name("name").or_else(|| child.child(0))
                            {
                                let var_name = node_text(name_node, source).to_string();
                                // The initializer/value is a sibling after the '='
                                // In Kotlin tree-sitter, the value/expression follows the property_declaration's delegation_specifier or directly
                                // Check remaining children for call expressions
                                let mut inner_cursor = node.walk();
                                for sibling in node.children(&mut inner_cursor) {
                                    if let Some(resource_type) = self
                                        .get_resource_type_from_call_multilang(
                                            sibling, source, patterns,
                                        )
                                    {
                                        self.resources.push(DetectedResource {
                                            name: var_name.clone(),
                                            resource_type,
                                            line: node.start_position().row as u32 + 1,
                                            in_context_manager: in_cleanup,
                                        });
                                        break;
                                    }
                                }
                            }
                        }
                    }
                } else if node.kind() == "assignment" {
                    if let Some(left) = node.child_by_field_name("left").or_else(|| node.child(0)) {
                        if let Some(right) = node.child_by_field_name("right") {
                            let var_name = node_text(left, source).to_string();
                            if let Some(resource_type) =
                                self.get_resource_type_from_call_multilang(right, source, patterns)
                            {
                                self.resources.push(DetectedResource {
                                    name: var_name,
                                    resource_type,
                                    line: node.start_position().row as u32 + 1,
                                    in_context_manager: in_cleanup,
                                });
                            }
                        }
                    }
                }
            }
            Language::Swift => {
                // Swift: let handle = FileHandle(forReadingAtPath: path)!
                // property_declaration has pattern (name) and value (expression)
                if node.kind() == "property_declaration"
                    || node.kind() == "directly_assignable_expression"
                {
                    if let Some(pattern) = node
                        .child_by_field_name("pattern")
                        .or_else(|| node.child_by_field_name("name"))
                    {
                        let var_name = node_text(pattern, source).to_string();
                        // Check all children for call expressions (value may be force-unwrapped, etc.)
                        let mut cursor = node.walk();
                        for child in node.children(&mut cursor) {
                            if let Some(resource_type) =
                                self.get_resource_type_from_call_multilang(child, source, patterns)
                            {
                                self.resources.push(DetectedResource {
                                    name: var_name.clone(),
                                    resource_type,
                                    line: node.start_position().row as u32 + 1,
                                    in_context_manager: in_cleanup,
                                });
                                break;
                            }
                        }
                    }
                }
            }
            Language::Ocaml => {
                // OCaml: let ic = open_in path in ...
                // let_binding has pattern (value_name) and body (application / expression)
                if node.kind() == "let_binding" {
                    if let Some(pattern) = node.child_by_field_name("pattern") {
                        let var_name = node_text(pattern, source).to_string();
                        // Check body for resource creation
                        if let Some(body) = node.child_by_field_name("body") {
                            if let Some(resource_type) =
                                self.get_resource_type_from_call_multilang(body, source, patterns)
                            {
                                self.resources.push(DetectedResource {
                                    name: var_name,
                                    resource_type,
                                    line: node.start_position().row as u32 + 1,
                                    in_context_manager: in_cleanup,
                                });
                            }
                        }
                    }
                }
            }
            Language::Lua | Language::Luau => {
                // Lua/Luau: local f = io.open(path, "r")
                // assignment_statement has variable_list and expression_list
                // variable_declaration has assignment with variable_list and expression_list
                if let Some(right) = node
                    .child_by_field_name("values")
                    .or_else(|| node.child_by_field_name("right"))
                {
                    if let Some(left) = node
                        .child_by_field_name("variables")
                        .or_else(|| node.child_by_field_name("left"))
                        .or_else(|| node.child_by_field_name("name"))
                    {
                        // left is usually a variable_list containing identifier(s)
                        let var_name =
                            if left.kind() == "variable_list" || left.kind() == "identifier_list" {
                                left.child(0).map(|c| node_text(c, source).to_string())
                            } else {
                                Some(node_text(left, source).to_string())
                            };
                        if let Some(var_name) = var_name {
                            // right is usually an expression_list
                            let call_node = if right.kind() == "expression_list" {
                                right.child(0)
                            } else {
                                Some(right)
                            };
                            if let Some(call_node) = call_node {
                                if let Some(resource_type) = self
                                    .get_resource_type_from_call_multilang(
                                        call_node, source, patterns,
                                    )
                                {
                                    self.resources.push(DetectedResource {
                                        name: var_name,
                                        resource_type,
                                        line: node.start_position().row as u32 + 1,
                                        in_context_manager: in_cleanup,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                // Generic fallback: try left/right fields
                if let Some(left) = node.child_by_field_name("left") {
                    if let Some(right) = node.child_by_field_name("right") {
                        let var_name = node_text(left, source).to_string();
                        if let Some(resource_type) =
                            self.get_resource_type_from_call_multilang(right, source, patterns)
                        {
                            self.resources.push(DetectedResource {
                                name: var_name,
                                resource_type,
                                line: node.start_position().row as u32 + 1,
                                in_context_manager: in_cleanup,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Multi-language resource type detection from call expressions.
    fn get_resource_type_from_call_multilang(
        &self,
        node: Node,
        source: &[u8],
        patterns: &LangResourcePatterns,
    ) -> Option<String> {
        // Extract the function/method name from the call
        let func_name = extract_call_name(node, source)?;

        // Check against language-specific creator patterns
        for &(creator, rtype) in patterns.creators {
            if func_name == creator
                || func_name.ends_with(&format!("::{}", creator))
                || func_name.ends_with(&format!(".{}", creator))
            {
                return Some(rtype.to_string());
            }
        }

        // For C/C++: also check for new/malloc at the call level
        if matches!(self.lang, Language::C | Language::Cpp) {
            if node.kind() == "call_expression" {
                let text = node_text(node, source);
                for &(creator, rtype) in patterns.creators {
                    if text.starts_with(creator) {
                        return Some(rtype.to_string());
                    }
                }
            }
            // Check for `new` expressions in C++
            if node.kind() == "new_expression" {
                return Some("heap_object".to_string());
            }
        }

        // For Kotlin: check for constructor calls like BufferedReader(FileReader(path))
        if matches!(self.lang, Language::Kotlin) {
            // Kotlin constructors look like function calls in tree-sitter
            let text = node_text(node, source);
            for &(creator, rtype) in patterns.creators {
                if text.starts_with(creator) {
                    return Some(rtype.to_string());
                }
            }
        }

        // For Swift: check for constructor calls like FileHandle(forReadingAtPath: path)
        if matches!(self.lang, Language::Swift) {
            let text = node_text(node, source);
            for &(creator, rtype) in patterns.creators {
                if text.starts_with(creator) {
                    return Some(rtype.to_string());
                }
            }
            // Also check force-unwrap: FileHandle(...)!
            if node.kind() == "force_unwrap_expression" || node.kind() == "try_expression" {
                if let Some(child) = node.child(0) {
                    return self.get_resource_type_from_call_multilang(child, source, patterns);
                }
            }
        }

        // For OCaml: check for function application like `open_in path`
        if matches!(self.lang, Language::Ocaml) {
            // OCaml uses application nodes: (application function: (value_name) argument: ...)
            if node.kind() == "application" {
                if let Some(func_node) = node
                    .child_by_field_name("function")
                    .or_else(|| node.child(0))
                {
                    let func_text = node_text(func_node, source);
                    for &(creator, rtype) in patterns.creators {
                        if func_text == creator || func_text.ends_with(&format!(".{}", creator)) {
                            return Some(rtype.to_string());
                        }
                    }
                }
            }
            // Also check the raw text for patterns like `open_in`
            let text = node_text(node, source);
            let first_word = text.split_whitespace().next().unwrap_or("");
            for &(creator, rtype) in patterns.creators {
                if first_word == creator {
                    return Some(rtype.to_string());
                }
            }
        }

        // For Lua/Luau: check for method calls like io.open(path, "r")
        if matches!(self.lang, Language::Lua | Language::Luau) {
            let text = node_text(node, source);
            for &(creator, rtype) in patterns.creators {
                if text.starts_with(creator) {
                    return Some(rtype.to_string());
                }
            }
        }

        // For Java/C#: check for `new ClassName(...)` constructor calls
        if matches!(self.lang, Language::Java | Language::CSharp)
            && node.kind() == "object_creation_expression"
        {
            // Get the type name
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_name = node_text(type_node, source);
                for &(creator, rtype) in patterns.creators {
                    if type_name == creator || type_name.contains(creator) {
                        return Some(rtype.to_string());
                    }
                }
            }
        }

        None
    }
}

impl Default for ResourceDetector {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Leak Detection (TIGER-04)
// =============================================================================

/// Leak detector using CFG path analysis.
pub struct LeakDetector {
    /// Maximum paths to enumerate (TIGER-04)
    max_paths: usize,
    /// Paths enumerated so far
    paths_enumerated: usize,
    /// Whether we hit the limit
    hit_limit: bool,
}

impl LeakDetector {
    pub fn new() -> Self {
        Self {
            max_paths: MAX_PATHS,
            paths_enumerated: 0,
            hit_limit: false,
        }
    }

    /// Detect potential leaks using CFG path analysis.
    pub fn detect(
        &mut self,
        cfg: &SimpleCfg,
        resources: &[ResourceInfo],
        source: &[u8],
        show_paths: bool,
    ) -> Vec<LeakInfo> {
        let mut leaks = Vec::new();
        self.paths_enumerated = 0;
        self.hit_limit = false;

        for resource in resources {
            // Skip resources in context managers
            if resource.closed {
                continue;
            }

            // Find all paths from resource creation to exits
            let paths = self.enumerate_paths(cfg, resource, source);

            // Check if any path lacks a close
            for path in &paths {
                if !self.path_has_close(path, &resource.name) {
                    leaks.push(LeakInfo {
                        resource: resource.name.clone(),
                        line: resource.line,
                        paths: if show_paths {
                            Some(vec![self.format_path(path)])
                        } else {
                            None
                        },
                    });
                    break; // One leak path is enough per resource
                }
            }
        }

        leaks
    }

    /// Detect potential leaks using CFG path analysis (multi-language).
    /// Same logic as `detect` since the CFG is already language-aware.
    pub fn detect_multilang(
        &mut self,
        cfg: &SimpleCfg,
        resources: &[ResourceInfo],
        source: &[u8],
        show_paths: bool,
    ) -> Vec<LeakInfo> {
        self.detect(cfg, resources, source, show_paths)
    }

    /// Enumerate paths from resource creation to exits (TIGER-04: with limit).
    fn enumerate_paths(
        &mut self,
        cfg: &SimpleCfg,
        resource: &ResourceInfo,
        _source: &[u8],
    ) -> Vec<Vec<usize>> {
        let mut paths = Vec::new();

        // Find which block contains the resource creation
        let start_block = self.find_block_with_line(cfg, resource.line);
        if start_block.is_none() {
            return paths;
        }
        let start = start_block.unwrap();

        // DFS to find all paths to exit blocks
        for &exit_id in &cfg.exit_blocks {
            if self.hit_limit {
                break;
            }
            self.find_paths_dfs(cfg, start, exit_id, &mut Vec::new(), &mut paths);
        }

        paths
    }

    fn find_block_with_line(&self, cfg: &SimpleCfg, line: u32) -> Option<usize> {
        for (id, block) in &cfg.blocks {
            if block.lines.contains(&line) {
                return Some(*id);
            }
        }
        // Default to entry block if not found
        Some(cfg.entry_block)
    }

    fn find_paths_dfs(
        &mut self,
        cfg: &SimpleCfg,
        current: usize,
        target: usize,
        current_path: &mut Vec<usize>,
        paths: &mut Vec<Vec<usize>>,
    ) {
        // TIGER-04: Check path limit
        if self.paths_enumerated >= self.max_paths {
            self.hit_limit = true;
            return;
        }

        // Cycle detection
        if current_path.contains(&current) {
            return;
        }

        current_path.push(current);

        if current == target {
            paths.push(current_path.clone());
            self.paths_enumerated += 1;
        } else if let Some(block) = cfg.blocks.get(&current) {
            for &succ in &block.succs {
                self.find_paths_dfs(cfg, succ, target, current_path, paths);
                if self.hit_limit {
                    break;
                }
            }
        }

        current_path.pop();
    }

    fn path_has_close(&self, path: &[usize], resource_name: &str) -> bool {
        // This is a simplified check - a real implementation would track
        // the resource state through the CFG
        // For now, we assume the path doesn't have a close
        // (proper implementation would look for close calls in each block)
        let _ = (path, resource_name);
        false
    }

    fn format_path(&self, path: &[usize]) -> String {
        path.iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(" -> ")
    }
}

impl Default for LeakDetector {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Double-Close Detection
// =============================================================================

/// Double-close detector.
pub struct DoubleCloseDetector {
    lang: Language,
}

impl DoubleCloseDetector {
    pub fn new() -> Self {
        Self {
            lang: Language::Python,
        }
    }

    pub fn with_language(lang: Language) -> Self {
        Self { lang }
    }

    /// Detect double-close issues (legacy Python).
    pub fn detect(&self, func_node: Node, source: &[u8]) -> Vec<DoubleCloseInfo> {
        let mut issues = Vec::new();
        let mut close_sites: HashMap<String, Vec<u32>> = HashMap::new();

        self.find_closes(func_node, source, &mut close_sites);

        for (resource, lines) in close_sites {
            if lines.len() > 1 {
                issues.push(DoubleCloseInfo {
                    resource,
                    first_close: lines[0],
                    second_close: lines[1],
                });
            }
        }

        issues
    }

    /// Detect double-close issues with multi-language support.
    pub fn detect_multilang(&self, func_node: Node, source: &[u8]) -> Vec<DoubleCloseInfo> {
        let mut issues = Vec::new();
        let mut close_sites: HashMap<String, Vec<u32>> = HashMap::new();
        let patterns = get_resource_patterns(self.lang);

        self.find_closes_multilang(func_node, source, &mut close_sites, &patterns);

        for (resource, lines) in close_sites {
            if lines.len() > 1 {
                issues.push(DoubleCloseInfo {
                    resource,
                    first_close: lines[0],
                    second_close: lines[1],
                });
            }
        }

        issues
    }

    fn find_closes(&self, node: Node, source: &[u8], closes: &mut HashMap<String, Vec<u32>>) {
        if node.kind() == "call" {
            if let Some(func) = node.child_by_field_name("function") {
                if func.kind() == "attribute" {
                    if let Some(attr) = func.child_by_field_name("attribute") {
                        let method = node_text(attr, source);
                        if RESOURCE_CLOSERS.contains(&method) {
                            if let Some(obj) = func.child_by_field_name("object") {
                                let var_name = node_text(obj, source).to_string();
                                let line = node.start_position().row as u32 + 1;
                                closes.entry(var_name).or_default().push(line);
                            }
                        }
                    }
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.find_closes(child, source, closes);
        }
    }

    fn find_closes_multilang(
        &self,
        node: Node,
        source: &[u8],
        closes: &mut HashMap<String, Vec<u32>>,
        patterns: &LangResourcePatterns,
    ) {
        let kind = node.kind();
        // Check for method call patterns: obj.close(), obj.Close(), fclose(obj), etc.
        if kind == "call"
            || kind == "call_expression"
            || kind == "method_invocation"
            || kind == "invocation_expression"
        {
            if let Some((var_name, method)) = extract_close_call(node, source, self.lang) {
                if patterns.closers.contains(&method.as_str()) {
                    let line = node.start_position().row as u32 + 1;
                    closes.entry(var_name).or_default().push(line);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.find_closes_multilang(child, source, closes, patterns);
        }
    }
}

impl Default for DoubleCloseDetector {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Use-After-Close Detection
// =============================================================================

/// Use-after-close detector.
pub struct UseAfterCloseDetector {
    lang: Language,
}

impl UseAfterCloseDetector {
    pub fn new() -> Self {
        Self {
            lang: Language::Python,
        }
    }

    pub fn with_language(lang: Language) -> Self {
        Self { lang }
    }

    /// Detect use-after-close issues (legacy Python).
    pub fn detect(&self, func_node: Node, source: &[u8]) -> Vec<UseAfterCloseInfo> {
        let mut issues = Vec::new();
        let mut close_lines: HashMap<String, u32> = HashMap::new();
        let mut uses_after_close: Vec<(String, u32, u32)> = Vec::new();

        self.analyze(func_node, source, &mut close_lines, &mut uses_after_close);

        for (resource, close_line, use_line) in uses_after_close {
            issues.push(UseAfterCloseInfo {
                resource,
                close_line,
                use_line,
            });
        }

        issues
    }

    /// Detect use-after-close issues with multi-language support.
    pub fn detect_multilang(&self, func_node: Node, source: &[u8]) -> Vec<UseAfterCloseInfo> {
        let mut issues = Vec::new();
        let mut close_lines: HashMap<String, u32> = HashMap::new();
        let mut uses_after_close: Vec<(String, u32, u32)> = Vec::new();
        let patterns = get_resource_patterns(self.lang);

        self.analyze_multilang(
            func_node,
            source,
            &mut close_lines,
            &mut uses_after_close,
            &patterns,
        );

        for (resource, close_line, use_line) in uses_after_close {
            issues.push(UseAfterCloseInfo {
                resource,
                close_line,
                use_line,
            });
        }

        issues
    }

    fn analyze(
        &self,
        node: Node,
        source: &[u8],
        close_lines: &mut HashMap<String, u32>,
        uses_after: &mut Vec<(String, u32, u32)>,
    ) {
        let line = node.start_position().row as u32 + 1;

        if node.kind() == "call" {
            if let Some(func) = node.child_by_field_name("function") {
                if func.kind() == "attribute" {
                    if let Some(attr) = func.child_by_field_name("attribute") {
                        let method = node_text(attr, source);
                        if RESOURCE_CLOSERS.contains(&method) {
                            if let Some(obj) = func.child_by_field_name("object") {
                                let var_name = node_text(obj, source).to_string();
                                close_lines.insert(var_name, line);
                            }
                        } else if let Some(obj) = func.child_by_field_name("object") {
                            let var_name = node_text(obj, source).to_string();
                            if let Some(&close_line) = close_lines.get(&var_name) {
                                if line > close_line {
                                    uses_after.push((var_name, close_line, line));
                                }
                            }
                        }
                    }
                }
            }
        }

        if node.kind() == "attribute" {
            if let Some(obj) = node.child_by_field_name("object") {
                if obj.kind() == "identifier" {
                    let var_name = node_text(obj, source).to_string();
                    if let Some(&close_line) = close_lines.get(&var_name) {
                        if line > close_line {
                            uses_after.push((var_name, close_line, line));
                        }
                    }
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.analyze(child, source, close_lines, uses_after);
        }
    }

    fn analyze_multilang(
        &self,
        node: Node,
        source: &[u8],
        close_lines: &mut HashMap<String, u32>,
        uses_after: &mut Vec<(String, u32, u32)>,
        patterns: &LangResourcePatterns,
    ) {
        let line = node.start_position().row as u32 + 1;
        let kind = node.kind();

        // Check for close calls
        if kind == "call"
            || kind == "call_expression"
            || kind == "method_invocation"
            || kind == "invocation_expression"
        {
            if let Some((var_name, method)) = extract_close_call(node, source, self.lang) {
                if patterns.closers.contains(&method.as_str()) {
                    close_lines.insert(var_name, line);
                } else {
                    // Non-close method call on a variable - check if it's been closed
                    // Try to extract the object name
                    if let Some((obj_name, _)) = extract_close_call(node, source, self.lang) {
                        if let Some(&close_line) = close_lines.get(&obj_name) {
                            if line > close_line {
                                uses_after.push((obj_name, close_line, line));
                            }
                        }
                    }
                }
            }
        }

        // Check for member access on closed resources
        if kind == "attribute"
            || kind == "member_expression"
            || kind == "field_expression"
            || kind == "selector_expression"
        {
            if let Some(obj) = node
                .child_by_field_name("object")
                .or_else(|| node.child_by_field_name("operand"))
                .or_else(|| node.child(0))
            {
                if obj.kind() == "identifier" {
                    let var_name = node_text(obj, source).to_string();
                    if let Some(&close_line) = close_lines.get(&var_name) {
                        if line > close_line {
                            uses_after.push((var_name, close_line, line));
                        }
                    }
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.analyze_multilang(child, source, close_lines, uses_after, patterns);
        }
    }
}

impl Default for UseAfterCloseDetector {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Context Manager Suggestions
// =============================================================================

/// Suggest context manager usage for resources.
pub fn suggest_context_manager(resources: &[ResourceInfo]) -> Vec<ContextSuggestion> {
    resources
        .iter()
        .filter(|r| !r.closed) // Only suggest for non-context-managed resources
        .map(|r| {
            let suggestion = match r.resource_type.as_str() {
                "file" => format!("with open(...) as {}:", r.name),
                "connection" => format!("with connect(...) as {}:", r.name),
                "cursor" => format!("with connection.cursor() as {}:", r.name),
                "socket" => format!("with socket.socket(...) as {}:", r.name),
                _ => format!("with {} as {}:", r.resource_type, r.name),
            };
            ContextSuggestion {
                resource: r.name.clone(),
                suggestion,
            }
        })
        .collect()
}

/// Suggest cleanup patterns using language-appropriate idioms.
pub fn suggest_context_manager_multilang(
    resources: &[ResourceInfo],
    lang: Language,
) -> Vec<ContextSuggestion> {
    resources
        .iter()
        .filter(|r| !r.closed)
        .map(|r| {
            let suggestion = match lang {
                Language::Python => match r.resource_type.as_str() {
                    "file" => format!("with open(...) as {}:", r.name),
                    "connection" => format!("with connect(...) as {}:", r.name),
                    "cursor" => format!("with connection.cursor() as {}:", r.name),
                    "socket" => format!("with socket.socket(...) as {}:", r.name),
                    _ => format!("with {} as {}:", r.resource_type, r.name),
                },
                Language::Go => format!("defer {}.Close()", r.name),
                Language::Rust => format!("// {}: Drop trait handles cleanup automatically. Consider wrapping in a scope block.", r.name),
                Language::Java => match r.resource_type.as_str() {
                    "file_stream" | "reader" | "writer" | "scanner" | "stream" =>
                        format!("try ({} {} = ...) {{ ... }}", r.resource_type, r.name),
                    "connection" | "statement" =>
                        format!("try ({} {} = ...) {{ ... }}", r.resource_type, r.name),
                    _ => format!("try ({} {} = ...) {{ ... }}", r.resource_type, r.name),
                },
                Language::CSharp => format!("using (var {} = ...) {{ ... }}", r.name),
                Language::TypeScript | Language::JavaScript =>
                    format!("try {{ ... }} finally {{ {}.close(); }}", r.name),
                Language::C => match r.resource_type.as_str() {
                    "file" => format!("// Ensure fclose({}) on all paths", r.name),
                    "memory" => format!("// Ensure free({}) on all paths", r.name),
                    _ => format!("// Ensure cleanup of {} on all paths", r.name),
                },
                Language::Cpp => match r.resource_type.as_str() {
                    "heap_object" => format!("// Use std::unique_ptr or std::shared_ptr instead of raw new for {}", r.name),
                    "memory" => format!("// Use RAII wrapper or smart pointer for {}", r.name),
                    _ => format!("// Consider RAII wrapper for {}", r.name),
                },
                Language::Ruby => format!("File.open(...) do |{}| ... end", r.name),
                Language::Php => format!("// Ensure {}() cleanup in finally block", r.name),
                Language::Kotlin => format!("{}.use {{ {} -> ... }}", r.name, r.name),
                Language::Swift => format!("defer {{ {}.closeFile() }}", r.name),
                Language::Ocaml => format!("Fun.protect ~finally:(fun () -> close_in {}) (fun () -> ...)", r.name),
                Language::Lua | Language::Luau => format!("// Ensure {}:close() is called, consider pcall for cleanup", r.name),
                _ => format!("// Ensure {} is properly closed/released", r.name),
            };
            ContextSuggestion {
                resource: r.name.clone(),
                suggestion,
            }
        })
        .collect()
}

// =============================================================================
// Constraint Generation
// =============================================================================

/// Generate LLM-ready constraints from resource analysis.
pub fn generate_constraints(
    file: &str,
    function: Option<&str>,
    resources: &[ResourceInfo],
    leaks: &[LeakInfo],
    double_closes: &[DoubleCloseInfo],
    use_after_closes: &[UseAfterCloseInfo],
) -> Vec<ResourceConstraint> {
    let mut constraints = Vec::new();
    let context = function.unwrap_or("module").to_string();

    // Generate constraints for leaks
    for leak in leaks {
        constraints.push(ResourceConstraint {
            rule: format!(
                "Resource '{}' opened at line {} must be closed on all control flow paths",
                leak.resource, leak.line
            ),
            context: format!("{} in {}", context, file),
            confidence: 0.9,
        });
    }

    // Generate constraints for double-closes
    for dc in double_closes {
        constraints.push(ResourceConstraint {
            rule: format!(
                "Resource '{}' must not be closed twice (lines {} and {})",
                dc.resource, dc.first_close, dc.second_close
            ),
            context: format!("{} in {}", context, file),
            confidence: 0.95,
        });
    }

    // Generate constraints for use-after-close
    for uac in use_after_closes {
        constraints.push(ResourceConstraint {
            rule: format!(
                "Resource '{}' must not be used at line {} after being closed at line {}",
                uac.resource, uac.use_line, uac.close_line
            ),
            context: format!("{} in {}", context, file),
            confidence: 0.95,
        });
    }

    // General resource usage patterns
    for resource in resources {
        if !resource.closed {
            constraints.push(ResourceConstraint {
                rule: format!(
                    "Resource '{}' ({}) should use context manager pattern (with statement)",
                    resource.name, resource.resource_type
                ),
                context: format!("{} in {}", context, file),
                confidence: 0.85,
            });
        }
    }

    constraints
}

// =============================================================================
// Output Formatting
// =============================================================================

/// Format resources report as human-readable text.
pub fn format_resources_text(report: &ResourceReport) -> String {
    let mut lines = Vec::new();

    lines.push(format!("Resource Analysis: {}", report.file));
    lines.push(format!("Language: {}", report.language));
    if let Some(ref func) = report.function {
        lines.push(format!("Function: {}", func));
    }
    lines.push(String::new());

    // Resources
    lines.push(format!("Resources detected: {}", report.resources.len()));
    for r in &report.resources {
        let status = if r.closed { "closed" } else { "open" };
        lines.push(format!(
            "  - {}: {} at line {} [{}]",
            r.name, r.resource_type, r.line, status
        ));
    }
    lines.push(String::new());

    // Leaks
    if !report.leaks.is_empty() {
        lines.push(format!("Leaks found: {}", report.leaks.len()));
        for leak in &report.leaks {
            lines.push(format!("  - {} at line {}", leak.resource, leak.line));
            if let Some(ref paths) = leak.paths {
                for path in paths {
                    lines.push(format!("    Path: {}", path));
                }
            }
        }
    } else {
        lines.push("Leaks found: 0".to_string());
    }

    // Double closes
    if !report.double_closes.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "Double-close errors: {}",
            report.double_closes.len()
        ));
        for dc in &report.double_closes {
            lines.push(format!(
                "  - {}: first close at {}, second close at {}",
                dc.resource, dc.first_close, dc.second_close
            ));
        }
    }

    // Use after close
    if !report.use_after_closes.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "Use-after-close errors: {}",
            report.use_after_closes.len()
        ));
        for uac in &report.use_after_closes {
            lines.push(format!(
                "  - {}: closed at {}, used at {}",
                uac.resource, uac.close_line, uac.use_line
            ));
        }
    }

    // Suggestions
    if !report.suggestions.is_empty() {
        lines.push(String::new());
        lines.push(format!("Suggestions: {}", report.suggestions.len()));
        for s in &report.suggestions {
            lines.push(format!("  - {}: {}", s.resource, s.suggestion));
        }
    }

    // Constraints
    if !report.constraints.is_empty() {
        lines.push(String::new());
        lines.push(format!("Constraints: {}", report.constraints.len()));
        for c in &report.constraints {
            lines.push(format!("  - {} (confidence: {:.2})", c.rule, c.confidence));
        }
    }

    // Summary
    lines.push(String::new());
    lines.push("Summary:".to_string());
    lines.push(format!(
        "  resources_detected: {}",
        report.summary.resources_detected
    ));
    lines.push(format!("  leaks_found: {}", report.summary.leaks_found));
    lines.push(format!(
        "  double_closes_found: {}",
        report.summary.double_closes_found
    ));
    lines.push(format!(
        "  use_after_closes_found: {}",
        report.summary.use_after_closes_found
    ));
    lines.push(String::new());
    lines.push(format!(
        "Analysis completed in {}ms",
        report.analysis_time_ms
    ));

    lines.join("\n")
}

// =============================================================================
// Helper Functions
// =============================================================================

fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract the function/method name from a call expression node.
/// Works across languages by checking various call node structures.
fn extract_call_name(node: Node, source: &[u8]) -> Option<String> {
    // Handle different call expression kinds across languages
    match node.kind() {
        // Python, JS/TS, Java, C#, Ruby, PHP
        "call" | "call_expression" | "method_invocation" | "invocation_expression" => {
            if let Some(func) = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("name"))
                .or_else(|| node.child_by_field_name("method"))
            {
                let func_text = node_text(func, source);
                // Extract just the function name from attribute/member access
                let func_name = func_text
                    .split('.')
                    .next_back()
                    .unwrap_or(func_text)
                    .rsplit("::")
                    .next()
                    .unwrap_or(func_text);
                return Some(func_name.to_string());
            }
            // For C/C++ call_expression, first child is the function
            if let Some(first_child) = node.child(0) {
                let text = node_text(first_child, source);
                let name = text
                    .split('.')
                    .next_back()
                    .unwrap_or(text)
                    .rsplit("::")
                    .next()
                    .unwrap_or(text);
                return Some(name.to_string());
            }
        }
        // Go: selector_expression.arguments
        "composite_literal" => {
            // Go: Type{} literal
        }
        _ => {}
    }

    // Fallback: check the whole node text for common patterns
    let text = node_text(node, source);
    if text.contains('(') {
        let name_part = text.split('(').next()?;
        let func_name = name_part
            .split('.')
            .next_back()
            .unwrap_or(name_part)
            .rsplit("::")
            .next()
            .unwrap_or(name_part)
            .trim();
        if !func_name.is_empty() {
            return Some(func_name.to_string());
        }
    }

    None
}

/// Extract the variable name from a C/C++ declarator (handles pointer_declarator, etc.)
fn extract_c_declarator_name(declarator: Node, source: &[u8]) -> Option<String> {
    match declarator.kind() {
        "identifier" => Some(node_text(declarator, source).to_string()),
        "pointer_declarator" => {
            // *foo -> get the identifier inside
            let mut cursor = declarator.walk();
            for child in declarator.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return Some(node_text(child, source).to_string());
                }
                if child.kind() == "pointer_declarator" {
                    return extract_c_declarator_name(child, source);
                }
            }
            None
        }
        _ => Some(node_text(declarator, source).to_string()),
    }
}

/// Extract (object_name, method_name) from a close call like `f.close()` or `fclose(fp)`.
fn extract_close_call(node: Node, source: &[u8], lang: Language) -> Option<(String, String)> {
    match lang {
        Language::Python
        | Language::Ruby
        | Language::Java
        | Language::CSharp
        | Language::TypeScript
        | Language::JavaScript
        | Language::Scala
        | Language::Kotlin
        | Language::Swift => {
            // obj.method() pattern
            if let Some(func) = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("method"))
                .or_else(|| node.child_by_field_name("name"))
            {
                // Check for attribute/member access: obj.close()
                if func.kind() == "attribute"
                    || func.kind() == "member_expression"
                    || func.kind() == "selector_expression"
                    || func.kind() == "field_access"
                {
                    let obj = func.child_by_field_name("object").or_else(|| func.child(0));
                    let attr = func
                        .child_by_field_name("attribute")
                        .or_else(|| func.child_by_field_name("field"))
                        .or_else(|| func.child_by_field_name("name"));

                    if let (Some(obj), Some(attr)) = (obj, attr) {
                        let var_name = node_text(obj, source).to_string();
                        let method = node_text(attr, source).to_string();
                        return Some((var_name, method));
                    }
                }
            }
            None
        }
        Language::Go => {
            // Go: obj.Close() - selector_expression
            if let Some(func) = node.child_by_field_name("function") {
                if func.kind() == "selector_expression" {
                    if let Some(operand) = func.child_by_field_name("operand") {
                        if let Some(field) = func.child_by_field_name("field") {
                            let var_name = node_text(operand, source).to_string();
                            let method = node_text(field, source).to_string();
                            return Some((var_name, method));
                        }
                    }
                }
            }
            None
        }
        Language::C | Language::Cpp => {
            // C: fclose(fp) - the variable is the first argument
            if let Some(func) = node
                .child_by_field_name("function")
                .or_else(|| node.child(0))
            {
                let func_name = node_text(func, source).to_string();
                // Get first argument
                if let Some(args) = node.child_by_field_name("arguments") {
                    if let Some(first_arg) = args.child(1) {
                        // child(0) is usually '('
                        let var_name = node_text(first_arg, source).to_string();
                        return Some((var_name, func_name));
                    }
                }
            }
            None
        }
        _ => {
            // Generic: try obj.method() pattern
            if let Some(func) = node.child_by_field_name("function") {
                if let Some(obj) = func.child_by_field_name("object").or_else(|| func.child(0)) {
                    if let Some(attr) = func.child_by_field_name("attribute") {
                        let var_name = node_text(obj, source).to_string();
                        let method = node_text(attr, source).to_string();
                        return Some((var_name, method));
                    }
                }
            }
            None
        }
    }
}

// =============================================================================
// Multi-language CFG Builder
// =============================================================================

/// Build a simplified CFG from a function AST, using language-specific patterns.
pub fn build_cfg_multilang(func_node: Node, source: &[u8], lang: Language) -> SimpleCfg {
    let patterns = get_resource_patterns(lang);
    let mut cfg = SimpleCfg::new();
    let entry_id = cfg.new_block();
    cfg.entry_block = entry_id;

    if let Some(block) = cfg.blocks.get_mut(&entry_id) {
        block.is_entry = true;
    }

    // Find the function body - try all known body kinds
    let body = func_node
        .children(&mut func_node.walk())
        .find(|n| patterns.body_kinds.contains(&n.kind()));

    if let Some(body_node) = body {
        let exit_id =
            process_statements_multilang(&mut cfg, body_node, source, entry_id, &patterns);
        if let Some(exit) = exit_id {
            if !cfg.blocks.get(&exit).is_none_or(|b| b.is_exit) {
                cfg.mark_exit(exit);
            }
        }
    } else {
        // Empty function or body not found - try processing children directly
        cfg.mark_exit(entry_id);
    }

    cfg
}

fn process_statements_multilang(
    cfg: &mut SimpleCfg,
    node: Node,
    source: &[u8],
    mut current: usize,
    patterns: &LangResourcePatterns,
) -> Option<usize> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();

        if patterns.return_kinds.contains(&kind) {
            // Return/raise/throw statement
            let text = node_text(child, source).to_string();
            let line = child.start_position().row as u32 + 1;
            if let Some(block) = cfg.blocks.get_mut(&current) {
                block
                    .stmts
                    .push((child.start_byte(), child.end_byte(), kind.to_string(), text));
                block.lines.push(line);
            }
            cfg.mark_exit(current);
            return None;
        } else if patterns.if_kinds.contains(&kind) {
            // If statement - creates branches
            current = process_if_multilang(cfg, child, source, current, patterns)?;
        } else if patterns.loop_kinds.contains(&kind) {
            // Loop statement
            current = process_loop_multilang(cfg, child, source, current, patterns)?;
        } else if patterns.try_kinds.contains(&kind) {
            // Try/catch statement
            current = process_try_multilang(cfg, child, source, current, patterns)?;
        } else if patterns.cleanup_block_kinds.contains(&kind) {
            // Context manager / defer / using
            current = process_cleanup_block_multilang(cfg, child, source, current, patterns)?;
        } else {
            // Regular statement
            let text = node_text(child, source).to_string();
            let line = child.start_position().row as u32 + 1;
            if let Some(block) = cfg.blocks.get_mut(&current) {
                block
                    .stmts
                    .push((child.start_byte(), child.end_byte(), kind.to_string(), text));
                block.lines.push(line);
            }
        }
    }
    Some(current)
}

fn process_if_multilang(
    cfg: &mut SimpleCfg,
    node: Node,
    source: &[u8],
    current: usize,
    patterns: &LangResourcePatterns,
) -> Option<usize> {
    // Add condition to current block
    if let Some(cond) = node.child_by_field_name("condition") {
        let text = node_text(cond, source).to_string();
        let line = cond.start_position().row as u32 + 1;
        if let Some(block) = cfg.blocks.get_mut(&current) {
            block.stmts.push((
                cond.start_byte(),
                cond.end_byte(),
                "condition".to_string(),
                text,
            ));
            block.lines.push(line);
        }
    }

    let true_block = cfg.new_block();
    cfg.add_edge(current, true_block);

    // Find the body block
    let mut cursor = node.walk();
    let consequence = node
        .children(&mut cursor)
        .find(|n| patterns.body_kinds.contains(&n.kind()));
    let true_exit = if let Some(body) = consequence {
        process_statements_multilang(cfg, body, source, true_block, patterns)
    } else {
        Some(true_block)
    };

    // Find alternative (else/elif)
    let mut cursor = node.walk();
    let alternative = node
        .children(&mut cursor)
        .find(|n| n.kind() == "else_clause" || n.kind() == "elif_clause" || n.kind() == "else");

    let false_exit = if let Some(alt) = alternative {
        let false_block = cfg.new_block();
        cfg.add_edge(current, false_block);
        let alt_body = alt
            .children(&mut alt.walk())
            .find(|n| patterns.body_kinds.contains(&n.kind()));
        if let Some(alt_body) = alt_body {
            process_statements_multilang(cfg, alt_body, source, false_block, patterns)
        } else {
            Some(false_block)
        }
    } else {
        None
    };

    let merge = cfg.new_block();
    if let Some(te) = true_exit {
        cfg.add_edge(te, merge);
    }
    if let Some(fe) = false_exit {
        cfg.add_edge(fe, merge);
    }
    if alternative.is_none() {
        cfg.add_edge(current, merge);
    }

    Some(merge)
}

fn process_loop_multilang(
    cfg: &mut SimpleCfg,
    node: Node,
    source: &[u8],
    current: usize,
    patterns: &LangResourcePatterns,
) -> Option<usize> {
    let header = cfg.new_block();
    cfg.add_edge(current, header);

    if let Some(cond) = node.child_by_field_name("condition") {
        let text = node_text(cond, source).to_string();
        let line = cond.start_position().row as u32 + 1;
        if let Some(block) = cfg.blocks.get_mut(&header) {
            block.stmts.push((
                cond.start_byte(),
                cond.end_byte(),
                "loop_condition".to_string(),
                text,
            ));
            block.lines.push(line);
        }
    }

    let body_block = cfg.new_block();
    cfg.add_edge(header, body_block);

    let body = node
        .children(&mut node.walk())
        .find(|n| patterns.body_kinds.contains(&n.kind()));
    let body_exit = if let Some(body_node) = body {
        process_statements_multilang(cfg, body_node, source, body_block, patterns)
    } else {
        Some(body_block)
    };

    if let Some(be) = body_exit {
        cfg.add_edge(be, header);
    }

    let exit = cfg.new_block();
    cfg.add_edge(header, exit);
    Some(exit)
}

fn process_try_multilang(
    cfg: &mut SimpleCfg,
    node: Node,
    source: &[u8],
    current: usize,
    patterns: &LangResourcePatterns,
) -> Option<usize> {
    let try_block = cfg.new_block();
    cfg.add_edge(current, try_block);

    let try_body = node
        .children(&mut node.walk())
        .find(|n| patterns.body_kinds.contains(&n.kind()));
    let try_exit = if let Some(body) = try_body {
        process_statements_multilang(cfg, body, source, try_block, patterns)
    } else {
        Some(try_block)
    };

    let mut cursor = node.walk();
    let mut handler_exits = Vec::new();
    for child in node.children(&mut cursor) {
        let ck = child.kind();
        if ck == "except_clause" || ck == "catch_clause" || ck == "rescue" {
            let handler_block = cfg.new_block();
            cfg.add_edge(try_block, handler_block);
            if let Some(block) = cfg.blocks.get_mut(&try_block) {
                block.exception_handlers.push(handler_block);
            }
            let handler_body = child
                .children(&mut child.walk())
                .find(|n| patterns.body_kinds.contains(&n.kind()));
            if let Some(hb) = handler_body {
                if let Some(exit) =
                    process_statements_multilang(cfg, hb, source, handler_block, patterns)
                {
                    handler_exits.push(exit);
                }
            } else {
                handler_exits.push(handler_block);
            }
        }
    }

    let finally_clause = node
        .children(&mut node.walk())
        .find(|n| n.kind() == "finally_clause" || n.kind() == "finally");

    let merge = cfg.new_block();
    if let Some(te) = try_exit {
        if let Some(finally) = finally_clause {
            let finally_block = cfg.new_block();
            cfg.add_edge(te, finally_block);
            let finally_body = finally
                .children(&mut finally.walk())
                .find(|n| patterns.body_kinds.contains(&n.kind()));
            if let Some(fb) = finally_body {
                if let Some(exit) =
                    process_statements_multilang(cfg, fb, source, finally_block, patterns)
                {
                    cfg.add_edge(exit, merge);
                }
            } else {
                cfg.add_edge(finally_block, merge);
            }
        } else {
            cfg.add_edge(te, merge);
        }
    }
    for he in handler_exits {
        cfg.add_edge(he, merge);
    }

    Some(merge)
}

fn process_cleanup_block_multilang(
    cfg: &mut SimpleCfg,
    node: Node,
    source: &[u8],
    current: usize,
    patterns: &LangResourcePatterns,
) -> Option<usize> {
    let text = node_text(node, source).to_string();
    let line = node.start_position().row as u32 + 1;
    if let Some(block) = cfg.blocks.get_mut(&current) {
        block.stmts.push((
            node.start_byte(),
            node.end_byte(),
            node.kind().to_string(),
            text,
        ));
        block.lines.push(line);
    }

    let body = node
        .children(&mut node.walk())
        .find(|n| patterns.body_kinds.contains(&n.kind()));
    if let Some(body_node) = body {
        process_statements_multilang(cfg, body_node, source, current, patterns)
    } else {
        Some(current)
    }
}

#[cfg(test)]
fn get_python_parser() -> PatternsResult<Parser> {
    get_parser_for_language(Language::Python)
}

/// Create a tree-sitter parser for the given language.
fn get_parser_for_language(lang: Language) -> PatternsResult<Parser> {
    let mut parser = Parser::new();
    let ts_lang =
        ParserPool::get_ts_language(lang).ok_or_else(|| PatternsError::UnsupportedLanguage {
            language: lang.as_str().to_string(),
        })?;
    parser
        .set_language(&ts_lang)
        .map_err(|e| PatternsError::ParseError {
            file: PathBuf::from("<internal>"),
            message: format!("Failed to set {} language: {}", lang.as_str(), e),
        })?;
    Ok(parser)
}

/// Get the function name from a node, handling language-specific declarator patterns.
/// For C/C++, the name is nested inside a `function_declarator` child of the `declarator` field.
/// For OCaml, value_definition wraps let_binding which has the pattern field.
fn get_function_name_from_node(
    node: Node,
    source: &[u8],
    patterns: &LangResourcePatterns,
) -> Option<String> {
    // OCaml: value_definition wraps let_binding(s)
    if node.kind() == "value_definition" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "let_binding" {
                if let Some(pattern) = child.child_by_field_name("pattern") {
                    return Some(node_text(pattern, source).to_string());
                }
            }
        }
        return None;
    }

    // First try the standard name field
    if let Some(name_node) = node.child_by_field_name(patterns.name_field) {
        // For C/C++, the "declarator" field contains a function_declarator
        // which itself has a "declarator" field containing the actual identifier
        if name_node.kind() == "function_declarator" {
            if let Some(inner) = name_node.child_by_field_name("declarator") {
                return Some(node_text(inner, source).to_string());
            }
        }
        // For pointer_declarator -> function_declarator pattern
        if name_node.kind() == "pointer_declarator" {
            let mut cursor = name_node.walk();
            for child in name_node.children(&mut cursor) {
                if child.kind() == "function_declarator" {
                    if let Some(inner) = child.child_by_field_name("declarator") {
                        return Some(node_text(inner, source).to_string());
                    }
                }
            }
        }
        return Some(node_text(name_node, source).to_string());
    }
    None
}

#[cfg(test)]
fn find_function_node<'a>(
    tree: &'a tree_sitter::Tree,
    function_name: &str,
    source: &[u8],
) -> Option<Node<'a>> {
    let root = tree.root_node();
    // Use Python patterns as default for backward compatibility
    let patterns = get_resource_patterns(Language::Python);
    find_function_recursive(root, function_name, source, &patterns)
}

fn find_function_node_multilang<'a>(
    tree: &'a tree_sitter::Tree,
    function_name: &str,
    source: &[u8],
    lang: Language,
) -> Option<Node<'a>> {
    let root = tree.root_node();
    let patterns = get_resource_patterns(lang);
    find_function_recursive(root, function_name, source, &patterns)
}

fn find_function_recursive<'a>(
    node: Node<'a>,
    function_name: &str,
    source: &[u8],
    patterns: &LangResourcePatterns,
) -> Option<Node<'a>> {
    let kind = node.kind();
    if patterns.function_kinds.contains(&kind) {
        if let Some(name) = get_function_name_from_node(node, source, patterns) {
            if name == function_name {
                return Some(node);
            }
        }
    }

    // Check for arrow functions in variable declarations (TS/JS pattern):
    // lexical_declaration / variable_declaration -> variable_declarator -> name + value(arrow_function)
    if matches!(kind, "lexical_declaration" | "variable_declaration") {
        let mut decl_cursor = node.walk();
        for child in node.children(&mut decl_cursor) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let var_name = name_node.utf8_text(source).unwrap_or("");
                    if var_name == function_name {
                        if let Some(value_node) = child.child_by_field_name("value") {
                            if matches!(
                                value_node.kind(),
                                "arrow_function"
                                    | "function"
                                    | "function_expression"
                                    | "generator_function"
                            ) {
                                return Some(value_node);
                            }
                        }
                    }
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_function_recursive(child, function_name, source, patterns) {
            return Some(found);
        }
    }

    None
}

fn find_all_functions_multilang<'a>(
    tree: &'a tree_sitter::Tree,
    source: &[u8],
    lang: Language,
) -> Vec<(String, Node<'a>)> {
    let mut functions = Vec::new();
    let patterns = get_resource_patterns(lang);
    collect_functions(tree.root_node(), source, &mut functions, &patterns);
    functions
}

fn collect_functions<'a>(
    node: Node<'a>,
    source: &[u8],
    functions: &mut Vec<(String, Node<'a>)>,
    patterns: &LangResourcePatterns,
) {
    let kind = node.kind();
    if patterns.function_kinds.contains(&kind) {
        if let Some(name) = get_function_name_from_node(node, source, patterns) {
            functions.push((name, node));
        }
    }

    // Check for arrow functions in variable declarations (TS/JS pattern):
    // lexical_declaration / variable_declaration -> variable_declarator -> name + value(arrow_function)
    if matches!(kind, "lexical_declaration" | "variable_declaration") {
        let mut decl_cursor = node.walk();
        for child in node.children(&mut decl_cursor) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Some(value_node) = child.child_by_field_name("value") {
                        if matches!(
                            value_node.kind(),
                            "arrow_function"
                                | "function"
                                | "function_expression"
                                | "generator_function"
                        ) {
                            let var_name = name_node.utf8_text(source).unwrap_or("").to_string();
                            functions.push((var_name, value_node));
                        }
                    }
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_functions(child, source, functions, patterns);
    }
}

// =============================================================================
// Main Analysis Function
// =============================================================================

fn analyze_function_with_lang(
    func_node: Node,
    source: &[u8],
    args: &ResourcesArgs,
    lang: Language,
) -> (
    Vec<ResourceInfo>,
    Vec<LeakInfo>,
    Vec<DoubleCloseInfo>,
    Vec<UseAfterCloseInfo>,
) {
    let check_leaks = args.check_leaks || args.check_all;
    let check_double_close = args.check_double_close || args.check_all;
    let check_use_after_close = args.check_use_after_close || args.check_all;
    // Detect resources
    let mut detector = ResourceDetector::with_language(lang);
    let resources = detector.detect_with_patterns(func_node, source);

    // Detect leaks
    let leaks = if check_leaks {
        let cfg = build_cfg_multilang(func_node, source, lang);
        let mut leak_detector = LeakDetector::new();
        leak_detector.detect_multilang(&cfg, &resources, source, args.show_paths)
    } else {
        Vec::new()
    };

    // Detect double-close
    let double_closes = if check_double_close {
        let detector = DoubleCloseDetector::with_language(lang);
        detector.detect_multilang(func_node, source)
    } else {
        Vec::new()
    };

    // Detect use-after-close
    let use_after_closes = if check_use_after_close {
        let detector = UseAfterCloseDetector::with_language(lang);
        detector.detect_multilang(func_node, source)
    } else {
        Vec::new()
    };

    (resources, leaks, double_closes, use_after_closes)
}

// =============================================================================
// Entry Point
// =============================================================================

/// Run the resources analysis command.
pub fn run(args: ResourcesArgs, global_format: GlobalOutputFormat) -> anyhow::Result<()> {
    let start_time = Instant::now();

    // Validate path
    let path = if let Some(ref root) = args.project_root {
        validate_file_path_in_project(&args.file, root)?
    } else {
        validate_file_path(&args.file)?
    };

    // Read file
    let source = read_file_safe(&path)?;
    let source_bytes = source.as_bytes();

    // Detect language (multi-language support)
    let lang: Language = match args.lang {
        Some(l) => l,
        None => Language::from_path(&path).ok_or_else(|| {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("unknown")
                .to_string();
            PatternsError::UnsupportedLanguage { language: ext }
        })?,
    };

    // Parse file with language-appropriate parser
    let mut parser = get_parser_for_language(lang)?;
    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| PatternsError::ParseError {
            file: path.clone(),
            message: format!("Failed to parse {} file", lang.as_str()),
        })?;

    // Collect results
    let mut all_resources = Vec::new();
    let mut all_leaks = Vec::new();
    let mut all_double_closes = Vec::new();
    let mut all_use_after_closes = Vec::new();

    if let Some(ref func_name) = args.function {
        // Analyze specific function
        if let Some(func_node) = find_function_node_multilang(&tree, func_name, source_bytes, lang)
        {
            let (resources, leaks, double_closes, use_after_closes) =
                analyze_function_with_lang(func_node, source_bytes, &args, lang);
            all_resources = resources;
            all_leaks = leaks;
            all_double_closes = double_closes;
            all_use_after_closes = use_after_closes;
        } else {
            return Err(PatternsError::FunctionNotFound {
                function: func_name.clone(),
                file: path.clone(),
            }
            .into());
        }
    } else {
        // Analyze all functions
        let functions = find_all_functions_multilang(&tree, source_bytes, lang);
        for (_name, func_node) in functions {
            let (resources, leaks, double_closes, use_after_closes) =
                analyze_function_with_lang(func_node, source_bytes, &args, lang);
            all_resources.extend(resources);
            all_leaks.extend(leaks);
            all_double_closes.extend(double_closes);
            all_use_after_closes.extend(use_after_closes);
        }
    }

    // Generate suggestions
    let suggestions = if args.suggest_context {
        suggest_context_manager_multilang(&all_resources, lang)
    } else {
        Vec::new()
    };

    // Generate constraints
    let constraints = if args.constraints {
        generate_constraints(
            path.to_str().unwrap_or(""),
            args.function.as_deref(),
            &all_resources,
            &all_leaks,
            &all_double_closes,
            &all_use_after_closes,
        )
    } else {
        Vec::new()
    };

    // Build summary
    let summary = ResourceSummary {
        resources_detected: all_resources.len() as u32,
        leaks_found: all_leaks.len() as u32,
        double_closes_found: all_double_closes.len() as u32,
        use_after_closes_found: all_use_after_closes.len() as u32,
    };

    let elapsed_ms = start_time.elapsed().as_millis() as u64;

    // Build report
    let report = ResourceReport {
        file: path.to_string_lossy().to_string(),
        language: lang.as_str().to_string(),
        function: args.function.clone(),
        resources: all_resources,
        leaks: all_leaks,
        double_closes: all_double_closes,
        use_after_closes: all_use_after_closes,
        suggestions,
        constraints,
        summary,
        analysis_time_ms: elapsed_ms,
    };

    // Output: global -f flag takes priority over hidden --output-format
    let use_text = matches!(global_format, GlobalOutputFormat::Text)
        || matches!(args.output_format, OutputFormat::Text);
    let output = if use_text {
        format_resources_text(&report)
    } else {
        serde_json::to_string_pretty(&report)?
    };

    println!("{}", output);

    // Exit code 3 if issues found
    let has_issues = report.summary.leaks_found > 0
        || report.summary.double_closes_found > 0
        || report.summary.use_after_closes_found > 0;

    if has_issues {
        std::process::exit(3);
    }

    Ok(())
}

// =============================================================================
// L2 Integration API
// =============================================================================

/// Aggregated resource analysis results for L2 consumption.
///
/// Each finding is paired with the function name where it was detected.
/// This avoids requiring callers to handle tree-sitter nodes directly.
pub struct ResourceAnalysisResults {
    /// Detected leaks: `(function_name, LeakInfo)`.
    pub leaks: Vec<(String, LeakInfo)>,
    /// Detected double-close issues: `(function_name, DoubleCloseInfo)`.
    pub double_closes: Vec<(String, DoubleCloseInfo)>,
    /// Detected use-after-close issues: `(function_name, UseAfterCloseInfo)`.
    pub use_after_closes: Vec<(String, UseAfterCloseInfo)>,
}

/// Analyze source code for resource lifecycle issues.
///
/// Parses the source with tree-sitter for the given language, finds all function
/// nodes, and runs the full resource analysis (leak, double-close, use-after-close)
/// on each function.
///
/// This is the primary entry point for L2 finding extractors that need resource
/// analysis without constructing `ResourcesArgs` or tree-sitter nodes themselves.
///
/// # Arguments
/// * `source` - Source code to analyze
/// * `lang` - Programming language for parsing
///
/// # Returns
/// `ResourceAnalysisResults` with all detected issues, or an error if parsing fails.
pub fn analyze_source_for_resource_issues(
    source: &str,
    lang: Language,
) -> PatternsResult<ResourceAnalysisResults> {
    let source_bytes = source.as_bytes();

    // Parse source with tree-sitter
    let mut parser = get_parser_for_language(lang)?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| PatternsError::ParseError {
            file: PathBuf::from("<in-memory>"),
            message: format!(
                "Failed to parse {} source for resource analysis",
                lang.as_str()
            ),
        })?;

    // Build args with all checks enabled
    let args = ResourcesArgs {
        file: PathBuf::from("<in-memory>"),
        function: None,
        lang: Some(lang),
        check_leaks: true,
        check_double_close: true,
        check_use_after_close: true,
        check_all: true,
        suggest_context: false,
        show_paths: false,
        constraints: false,
        summary: false,
        output_format: OutputFormat::Json,
        project_root: None,
    };

    let mut all_leaks = Vec::new();
    let mut all_double_closes = Vec::new();
    let mut all_use_after_closes = Vec::new();

    // Find all functions and analyze each
    let functions = find_all_functions_multilang(&tree, source_bytes, lang);
    for (func_name, func_node) in functions {
        let (_resources, leaks, double_closes, use_after_closes) =
            analyze_function_with_lang(func_node, source_bytes, &args, lang);

        for leak in leaks {
            all_leaks.push((func_name.clone(), leak));
        }
        for dc in double_closes {
            all_double_closes.push((func_name.clone(), dc));
        }
        for uac in use_after_closes {
            all_use_after_closes.push((func_name.clone(), uac));
        }
    }

    Ok(ResourceAnalysisResults {
        leaks: all_leaks,
        double_closes: all_double_closes,
        use_after_closes: all_use_after_closes,
    })
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_LEAKY_FUNCTION: &str = r#"
def leaky_function(path):
    f = open(path)
    if some_condition():
        return None
    content = f.read()
    f.close()
    return content
"#;

    const TEST_SAFE_WITH_CONTEXT: &str = r#"
def safe_with_context(path):
    with open(path) as f:
        return f.read()
"#;

    const TEST_DOUBLE_CLOSE: &str = r#"
def double_close(path):
    f = open(path)
    content = f.read()
    f.close()
    f.close()
    return content
"#;

    const TEST_USE_AFTER_CLOSE: &str = r#"
def use_after_close(path):
    f = open(path)
    f.close()
    content = f.read()
    return content
"#;

    #[test]
    fn test_resource_creators_constant() {
        assert!(RESOURCE_CREATORS.contains(&"open"));
        assert!(RESOURCE_CREATORS.contains(&"socket"));
        assert!(RESOURCE_CREATORS.contains(&"connect"));
        assert!(RESOURCE_CREATORS.contains(&"cursor"));
    }

    #[test]
    fn test_resource_closers_constant() {
        assert!(RESOURCE_CLOSERS.contains(&"close"));
        assert!(RESOURCE_CLOSERS.contains(&"shutdown"));
        assert!(RESOURCE_CLOSERS.contains(&"disconnect"));
    }

    #[test]
    fn test_max_paths_constant() {
        assert_eq!(MAX_PATHS, 1000);
    }

    #[test]
    fn test_resource_detector_finds_open() {
        let mut parser = get_python_parser().unwrap();
        let tree = parser.parse(TEST_LEAKY_FUNCTION, None).unwrap();
        let source = TEST_LEAKY_FUNCTION.as_bytes();

        let func_node = find_function_node(&tree, "leaky_function", source).unwrap();
        let mut detector = ResourceDetector::new();
        let resources = detector.detect(func_node, source);

        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].name, "f");
        assert_eq!(resources[0].resource_type, "file");
        assert!(!resources[0].closed);
    }

    #[test]
    fn test_resource_detector_context_manager() {
        let mut parser = get_python_parser().unwrap();
        let tree = parser.parse(TEST_SAFE_WITH_CONTEXT, None).unwrap();
        let source = TEST_SAFE_WITH_CONTEXT.as_bytes();

        let func_node = find_function_node(&tree, "safe_with_context", source).unwrap();
        let mut detector = ResourceDetector::new();
        let resources = detector.detect(func_node, source);

        assert_eq!(resources.len(), 1);
        assert!(
            resources[0].closed,
            "Context manager resource should be marked as closed"
        );
    }

    #[test]
    fn test_double_close_detector() {
        let mut parser = get_python_parser().unwrap();
        let tree = parser.parse(TEST_DOUBLE_CLOSE, None).unwrap();
        let source = TEST_DOUBLE_CLOSE.as_bytes();

        let func_node = find_function_node(&tree, "double_close", source).unwrap();
        let detector = DoubleCloseDetector::new();
        let issues = detector.detect(func_node, source);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].resource, "f");
    }

    #[test]
    fn test_use_after_close_detector() {
        let mut parser = get_python_parser().unwrap();
        let tree = parser.parse(TEST_USE_AFTER_CLOSE, None).unwrap();
        let source = TEST_USE_AFTER_CLOSE.as_bytes();

        let func_node = find_function_node(&tree, "use_after_close", source).unwrap();
        let detector = UseAfterCloseDetector::new();
        let issues = detector.detect(func_node, source);

        assert!(!issues.is_empty());
        assert_eq!(issues[0].resource, "f");
    }

    #[test]
    fn test_suggest_context_manager() {
        let resources = vec![ResourceInfo {
            name: "f".to_string(),
            resource_type: "file".to_string(),
            line: 2,
            closed: false,
        }];

        let suggestions = suggest_context_manager(&resources);
        assert_eq!(suggestions.len(), 1);
        assert!(suggestions[0].suggestion.contains("with open"));
    }

    #[test]
    fn test_generate_constraints_for_leak() {
        let resources = vec![ResourceInfo {
            name: "f".to_string(),
            resource_type: "file".to_string(),
            line: 2,
            closed: false,
        }];
        let leaks = vec![LeakInfo {
            resource: "f".to_string(),
            line: 2,
            paths: None,
        }];

        let constraints =
            generate_constraints("test.py", Some("test_func"), &resources, &leaks, &[], &[]);

        assert!(!constraints.is_empty());
        assert!(constraints[0].rule.contains("must be closed"));
    }

    #[test]
    fn test_leak_detector_path_limit() {
        let detector = LeakDetector::new();
        assert_eq!(detector.max_paths, MAX_PATHS);
    }

    #[test]
    fn test_cfg_builder_basic() {
        let mut parser = get_python_parser().unwrap();
        let source = r#"
def simple():
    x = 1
    return x
"#;
        let tree = parser.parse(source, None).unwrap();
        let func_node = find_function_node(&tree, "simple", source.as_bytes()).unwrap();
        let cfg = build_cfg(func_node, source.as_bytes());

        assert!(!cfg.blocks.is_empty());
        assert!(!cfg.exit_blocks.is_empty());
    }

    #[test]
    fn test_cfg_builder_with_if() {
        let mut parser = get_python_parser().unwrap();
        let source = r#"
def with_if(x):
    if x > 0:
        return x
    return -x
"#;
        let tree = parser.parse(source, None).unwrap();
        let func_node = find_function_node(&tree, "with_if", source.as_bytes()).unwrap();
        let cfg = build_cfg(func_node, source.as_bytes());

        // Should have multiple blocks for the branching
        assert!(cfg.blocks.len() > 1);
    }

    #[test]
    fn test_format_resources_text() {
        let report = ResourceReport {
            file: "test.py".to_string(),
            language: "python".to_string(),
            function: Some("test".to_string()),
            resources: vec![ResourceInfo {
                name: "f".to_string(),
                resource_type: "file".to_string(),
                line: 2,
                closed: false,
            }],
            leaks: vec![],
            double_closes: vec![],
            use_after_closes: vec![],
            suggestions: vec![],
            constraints: vec![],
            summary: ResourceSummary::default(),
            analysis_time_ms: 10,
        };

        let text = format_resources_text(&report);
        assert!(text.contains("Resource Analysis: test.py"));
        assert!(text.contains("Function: test"));
        assert!(text.contains("file"));
    }

    #[test]
    fn test_find_ts_arrow_function_resources() {
        let ts_source = r#"
const getDuration = (start: Date, end: Date): number => {
    const conn = createConnection();
    const result = end.getTime() - start.getTime();
    conn.close();
    return result;
};

function regularFunc(x: number): number {
    return x * 2;
}
"#;
        let tree = tldr_core::ast::parser::parse(ts_source, Language::TypeScript).unwrap();
        let source_bytes = ts_source.as_bytes();

        // Regular function should be found
        let regular =
            find_function_node_multilang(&tree, "regularFunc", source_bytes, Language::TypeScript);
        assert!(regular.is_some(), "Should find regular TS function");

        // Arrow function assigned to const should also be found
        let arrow =
            find_function_node_multilang(&tree, "getDuration", source_bytes, Language::TypeScript);
        assert!(
            arrow.is_some(),
            "Should find TS arrow function 'getDuration'"
        );
    }

    #[test]
    fn test_resources_args_lang_flag() {
        // Verify ResourcesArgs has a lang field of type Option<Language> (not language: String)
        let args = ResourcesArgs {
            file: PathBuf::from("src/db.go"),
            function: None,
            lang: Some(Language::Go),
            check_leaks: true,
            check_double_close: false,
            check_use_after_close: false,
            check_all: false,
            suggest_context: false,
            show_paths: false,
            constraints: false,
            summary: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };
        assert_eq!(args.lang, Some(Language::Go));

        // Also test None case (auto-detect)
        let args_auto = ResourcesArgs {
            file: PathBuf::from("src/db.py"),
            function: None,
            lang: None,
            check_leaks: true,
            check_double_close: false,
            check_use_after_close: false,
            check_all: false,
            suggest_context: false,
            show_paths: false,
            constraints: false,
            summary: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };
        assert_eq!(args_auto.lang, None);
    }
}
