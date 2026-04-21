//! Taint Analysis Types
//!
//! This module provides the core types for CFG-based taint analysis.
//! Taint analysis tracks how untrusted data flows through a program
//! to detect potential security vulnerabilities like SQL injection,
//! command injection, and code injection.
//!
//! # Types
//!
//! - `TaintSourceType` - Categorizes sources of untrusted input
//! - `TaintSinkType` - Categorizes dangerous operations (sinks)
//! - `SanitizerType` - Categorizes sanitization operations
//! - `TaintSource` - A detected source of tainted data
//! - `TaintSink` - A detected dangerous sink
//! - `TaintFlow` - A flow from source to sink (potential vulnerability)
//! - `TaintInfo` - Complete taint analysis result for a function
//!
//! # References
//! - session11-taint-spec.md

use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::types::{CfgInfo, RefType, VarRef};
use crate::Language;
use crate::TldrError;

/// Hard cap on worklist iterations to prevent infinite loops in taint analysis.
///
/// The computed max_iterations (blocks * vars) can be enormous for real-world
/// files with many blocks and variables. When the taint set oscillates (e.g.,
/// due to substring matching in stmt.contains()), the worklist never converges.
/// This cap ensures the analysis always terminates in bounded time.
const MAX_TAINT_ITERATIONS: usize = 1000;

// =============================================================================
// Enums - Taint Categories
// =============================================================================

/// Source of tainted (untrusted) data.
///
/// These represent entry points where external data enters the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaintSourceType {
    /// User input: `input()`, `raw_input()`
    UserInput,
    /// Standard input: `sys.stdin.read()`, `sys.stdin.readline()`
    Stdin,
    /// HTTP query/form parameters: `request.args`, `request.form`, `request.values`
    HttpParam,
    /// HTTP body data: `request.json`, `request.data`, `request.body`
    HttpBody,
    /// Environment variables: `os.environ`, `os.getenv()`
    EnvVar,
    /// File reads: `open().read()`, `pathlib.read_text()`
    FileRead,
}

/// Dangerous sink types where tainted data should not flow unsanitized.
///
/// These represent operations that can be exploited if fed untrusted data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaintSinkType {
    /// SQL queries: `cursor.execute()`, raw SQL execution
    SqlQuery,
    /// Code evaluation: `eval()`
    CodeEval,
    /// Code execution: `exec()`
    CodeExec,
    /// Code compilation: `compile()`
    CodeCompile,
    /// Shell command execution: `os.system()`, `subprocess.run()`
    ShellExec,
    /// File writes: `open(..., 'w')`, `.write_text()`
    FileWrite,
}

/// Sanitizer types that neutralize taint.
///
/// These represent operations that make tainted data safe for specific sinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizerType {
    /// Numeric conversion: `int()`, `float()`, `bool()` - safe for SQL
    Numeric,
    /// Shell escaping: `shlex.quote()` - safe for shell commands
    Shell,
    /// HTML escaping: `html.escape()`, `markupsafe.escape()` - safe for HTML output
    Html,
}

// =============================================================================
// Structs - Taint Data
// =============================================================================

/// A detected taint source - where untrusted data enters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintSource {
    /// Variable name that receives tainted data
    pub var: String,
    /// Line number of the source
    pub line: u32,
    /// Type of source
    pub source_type: TaintSourceType,
    /// Optional statement text for context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement: Option<String>,
}

/// A detected taint sink - dangerous operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintSink {
    /// Variable used in the sink
    pub var: String,
    /// Line number of the sink
    pub line: u32,
    /// Type of sink
    pub sink_type: TaintSinkType,
    /// Whether the variable is tainted at this sink (true = vulnerability)
    pub tainted: bool,
    /// Optional statement text for context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement: Option<String>,
}

/// A taint flow from source to sink (represents a potential vulnerability).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintFlow {
    /// The source of tainted data
    pub source: TaintSource,
    /// The sink where tainted data flows
    pub sink: TaintSink,
    /// Block IDs along the flow path from source to sink
    pub path: Vec<usize>,
}

/// Complete taint analysis result for a function.
///
/// Contains all detected sources, sinks, and flows, plus the taint state
/// at each CFG block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaintInfo {
    /// Function name
    pub function_name: String,
    /// Tainted variables at each block: block_id -> set of tainted variable names
    pub tainted_vars: HashMap<usize, HashSet<String>>,
    /// All detected taint sources
    pub sources: Vec<TaintSource>,
    /// All detected sinks (both tainted and untainted)
    pub sinks: Vec<TaintSink>,
    /// Flows from source to sink (vulnerabilities)
    pub flows: Vec<TaintFlow>,
    /// Variables that have been sanitized
    pub sanitized_vars: HashSet<String>,
    /// Convergence status: "converged" if the worklist reached a fixed point,
    /// "iteration_limit_reached" if analysis was capped at MAX_TAINT_ITERATIONS.
    #[serde(default = "default_convergence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub convergence: Option<String>,
}

fn default_convergence() -> Option<String> {
    None
}

// =============================================================================
// Implementations
// =============================================================================

impl TaintInfo {
    /// Create a new TaintInfo for a function with empty collections.
    pub fn new(function_name: impl Into<String>) -> Self {
        Self {
            function_name: function_name.into(),
            tainted_vars: HashMap::new(),
            sources: Vec::new(),
            sinks: Vec::new(),
            flows: Vec::new(),
            sanitized_vars: HashSet::new(),
            convergence: None,
        }
    }

    /// Check if a variable is tainted at a given block.
    ///
    /// Returns `false` if the block doesn't exist or the variable isn't tainted.
    pub fn is_tainted(&self, block_id: usize, var: &str) -> bool {
        self.tainted_vars
            .get(&block_id)
            .map(|vars| vars.contains(var))
            .unwrap_or(false)
    }

    /// Get all sinks where the variable is tainted (vulnerabilities).
    pub fn get_vulnerabilities(&self) -> Vec<&TaintSink> {
        self.sinks.iter().filter(|s| s.tainted).collect()
    }
}

// =============================================================================
// Helper Functions - Phase 2
// =============================================================================

/// Build predecessor map from CFG edges.
///
/// Returns a mapping from each block ID to its list of predecessor block IDs.
/// Every block is guaranteed to have an entry (even if empty).
pub fn build_predecessors(cfg: &CfgInfo) -> HashMap<usize, Vec<usize>> {
    let mut preds: HashMap<usize, Vec<usize>> = HashMap::new();

    // Initialize all blocks with empty predecessor lists
    for block in &cfg.blocks {
        preds.entry(block.id).or_default();
    }

    // Add predecessors from edges
    for edge in &cfg.edges {
        preds.entry(edge.to).or_default().push(edge.from);
    }

    preds
}

/// Build successor map from CFG edges.
///
/// Returns a mapping from each block ID to its list of successor block IDs.
/// Every block is guaranteed to have an entry (even if empty).
pub fn build_successors(cfg: &CfgInfo) -> HashMap<usize, Vec<usize>> {
    let mut succs: HashMap<usize, Vec<usize>> = HashMap::new();

    // Initialize all blocks with empty successor lists
    for block in &cfg.blocks {
        succs.entry(block.id).or_default();
    }

    // Add successors from edges
    for edge in &cfg.edges {
        succs.entry(edge.from).or_default().push(edge.to);
    }

    succs
}

/// Build line-to-block mapping from CFG.
///
/// Maps each line number to the block that contains it.
/// When blocks overlap (e.g., merge points within code blocks),
/// prefers LARGER blocks (actual code blocks over merge points).
/// For same-size blocks, prefers HIGHER block ID (branch bodies come after merge points).
///
/// This pattern is copied from reaching.rs:102-125 to handle overlapping blocks correctly.
pub fn build_line_to_block(cfg: &CfgInfo) -> HashMap<u32, usize> {
    let mut mapping: HashMap<u32, usize> = HashMap::new();

    // For each line, find the best block that contains it
    // We need to collect all lines first, then find the best block for each
    let mut all_lines: HashSet<u32> = HashSet::new();
    for block in &cfg.blocks {
        for line in block.lines.0..=block.lines.1 {
            all_lines.insert(line);
        }
    }

    for line in all_lines {
        let mut best_block: Option<(usize, u32)> = None; // (block_id, size)

        for block in &cfg.blocks {
            let (start, end) = block.lines;
            if line >= start && line <= end {
                let size = end - start + 1;
                // Prefer LARGER blocks (more likely to be actual code blocks)
                // For same size, prefer HIGHER block ID (branch bodies come after merge points)
                if best_block.is_none()
                    || size > best_block.unwrap().1
                    || (size == best_block.unwrap().1 && block.id > best_block.unwrap().0)
                {
                    best_block = Some((block.id, size));
                }
            }
        }

        if let Some((block_id, _)) = best_block {
            mapping.insert(line, block_id);
        }
    }

    mapping
}

/// Group VarRefs by their containing block.
///
/// Uses the line_to_block mapping to assign each VarRef to its block.
/// Refs within each block are sorted by line number.
/// VarRefs that don't map to any block are excluded.
pub fn build_refs_by_block<'a>(
    refs: &'a [VarRef],
    line_to_block: &HashMap<u32, usize>,
) -> HashMap<usize, Vec<&'a VarRef>> {
    let mut by_block: HashMap<usize, Vec<&VarRef>> = HashMap::new();

    for var_ref in refs {
        if let Some(&block_id) = line_to_block.get(&var_ref.line) {
            by_block.entry(block_id).or_default().push(var_ref);
        }
    }

    // Sort refs within each block by line number
    for refs in by_block.values_mut() {
        refs.sort_by_key(|r| r.line);
    }

    by_block
}

/// Validate CFG structure for taint analysis.
///
/// Checks:
/// - CFG has at least one block
/// - Entry block exists in the block list
/// - All edge endpoints reference valid block IDs
///
/// # Errors
///
/// Returns `TldrError::InvalidArgs` if validation fails.
pub fn validate_cfg(cfg: &CfgInfo) -> Result<(), TldrError> {
    // Check for empty CFG
    if cfg.blocks.is_empty() {
        return Err(TldrError::InvalidArgs {
            arg: "cfg".to_string(),
            message: "Empty CFG".to_string(),
            suggestion: None,
        });
    }

    // Collect all valid block IDs
    let block_ids: HashSet<usize> = cfg.blocks.iter().map(|b| b.id).collect();

    // Check entry block exists
    if !block_ids.contains(&cfg.entry_block) {
        return Err(TldrError::InvalidArgs {
            arg: "cfg".to_string(),
            message: format!("Entry block {} not in blocks", cfg.entry_block),
            suggestion: Some(format!(
                "Valid block IDs are: {:?}",
                block_ids.iter().collect::<Vec<_>>()
            )),
        });
    }

    // Check all edges reference valid blocks
    for edge in &cfg.edges {
        if !block_ids.contains(&edge.from) {
            return Err(TldrError::InvalidArgs {
                arg: "cfg".to_string(),
                message: format!(
                    "Edge references invalid source block: {} -> {}",
                    edge.from, edge.to
                ),
                suggestion: Some(format!(
                    "Valid block IDs are: {:?}",
                    block_ids.iter().collect::<Vec<_>>()
                )),
            });
        }
        if !block_ids.contains(&edge.to) {
            return Err(TldrError::InvalidArgs {
                arg: "cfg".to_string(),
                message: format!(
                    "Edge references invalid target block: {} -> {}",
                    edge.from, edge.to
                ),
                suggestion: Some(format!(
                    "Valid block IDs are: {:?}",
                    block_ids.iter().collect::<Vec<_>>()
                )),
            });
        }
    }

    Ok(())
}

// =============================================================================
// Pattern Matching - Phase 3
// =============================================================================

/// Language-specific taint analysis patterns.
///
/// Each language has its own set of source, sink, and sanitizer patterns.
/// Currently only Python patterns are defined; other languages fall back to Python.
pub struct LanguagePatterns {
    /// Regex patterns that identify taint sources and their source type.
    pub sources: Vec<(Regex, TaintSourceType)>,
    /// Regex patterns that identify taint sinks and their sink type.
    pub sinks: Vec<(Regex, TaintSinkType)>,
    /// Regex patterns that identify sanitizer calls and their sanitizer type.
    pub sanitizers: Vec<(Regex, SanitizerType)>,
}

lazy_static! {
    /// Python-specific taint patterns.
    static ref PYTHON_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // UserInput: input() function
            (Regex::new(r"\binput\s*\(").unwrap(), TaintSourceType::UserInput),
            // HttpParam: request.args, request.form, request.values, request.cookies, request.headers
            (Regex::new(r"request\.(args|form|json|data|values|cookies|headers)").unwrap(), TaintSourceType::HttpParam),
            // HttpBody: request.get_json()
            (Regex::new(r"request\.get_json\s*\(").unwrap(), TaintSourceType::HttpBody),
            // Stdin: sys.stdin
            (Regex::new(r"sys\.stdin").unwrap(), TaintSourceType::Stdin),
            // EnvVar: os.environ, os.getenv
            (Regex::new(r"os\.(environ|getenv)").unwrap(), TaintSourceType::EnvVar),
            // FileRead: .read(), .readlines(), .readline()
            (Regex::new(r"\.(read|readlines|readline)\s*\(").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // SqlQuery: .execute(), .executemany()
            (Regex::new(r"\.(execute|executemany)\s*\(").unwrap(), TaintSinkType::SqlQuery),
            // CodeEval: eval()
            (Regex::new(r"\beval\s*\(").unwrap(), TaintSinkType::CodeEval),
            // CodeExec: exec()
            (Regex::new(r"\bexec\s*\(").unwrap(), TaintSinkType::CodeExec),
            // CodeCompile: compile()
            (Regex::new(r"\bcompile\s*\(").unwrap(), TaintSinkType::CodeCompile),
            // ShellExec: subprocess.run, subprocess.call, subprocess.Popen, subprocess.check_output
            (Regex::new(r"subprocess\.(run|call|Popen|check_output)\s*\(").unwrap(), TaintSinkType::ShellExec),
            // ShellExec: os.system, os.popen, os.spawn*
            (Regex::new(r"os\.(system|popen|spawn\w*)\s*\(").unwrap(), TaintSinkType::ShellExec),
            // FileWrite: .write()
            (Regex::new(r"\.write\s*\(").unwrap(), TaintSinkType::FileWrite),
        ],
        sanitizers: vec![
            // Numeric: int(), float(), bool()
            (Regex::new(r"\b(int|float|bool)\s*\(").unwrap(), SanitizerType::Numeric),
            // Shell: shlex.quote, pipes.quote
            (Regex::new(r"(shlex|pipes)\.quote\s*\(").unwrap(), SanitizerType::Shell),
            // Html: html.escape, markupsafe.escape, cgi.escape
            (Regex::new(r"(html|markupsafe|cgi)\.escape\s*\(").unwrap(), SanitizerType::Html),
        ],
    };
}

lazy_static! {
    /// TypeScript/JavaScript taint patterns.
    static ref TYPESCRIPT_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // HttpBody: req.body
            (Regex::new(r"req\.body").unwrap(), TaintSourceType::HttpBody),
            // HttpParam: req.params, req.query, req.cookies, req.headers
            (Regex::new(r"req\.(params|query|cookies|headers)").unwrap(), TaintSourceType::HttpParam),
            // EnvVar: process.env
            (Regex::new(r"process\.env").unwrap(), TaintSourceType::EnvVar),
            // Stdin: process.stdin
            (Regex::new(r"process\.stdin").unwrap(), TaintSourceType::Stdin),
            // UserInput: readline()
            (Regex::new(r"readline\s*\(").unwrap(), TaintSourceType::UserInput),
            // FileRead: .read(), .readFile
            (Regex::new(r"\.(read|readFile)\s*\(").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // CodeEval: eval()
            (Regex::new(r"\beval\s*\(").unwrap(), TaintSinkType::CodeEval),
            // CodeEval: new Function()
            (Regex::new(r"new\s+Function\s*\(").unwrap(), TaintSinkType::CodeEval),
            // ShellExec: child_process.exec/spawn/execSync/execFile
            (Regex::new(r"child_process\.(exec|spawn|execSync|execFile)\s*\(").unwrap(), TaintSinkType::ShellExec),
            // ShellExec: execSync()
            (Regex::new(r"\bexecSync\s*\(").unwrap(), TaintSinkType::ShellExec),
            // FileWrite: innerHTML = (XSS)
            (Regex::new(r"\.innerHTML\s*=").unwrap(), TaintSinkType::FileWrite),
            // FileWrite: document.write (XSS)
            (Regex::new(r"document\.write\s*\(").unwrap(), TaintSinkType::FileWrite),
            // SqlQuery: .query(), .execute()
            (Regex::new(r"\.(query|execute)\s*\(").unwrap(), TaintSinkType::SqlQuery),
        ],
        sanitizers: vec![
            // Numeric: parseInt, Number, parseFloat
            (Regex::new(r"\b(parseInt|Number|parseFloat)\s*\(").unwrap(), SanitizerType::Numeric),
            // Html: encodeURIComponent, DOMPurify.sanitize
            (Regex::new(r"(encodeURIComponent|DOMPurify\.sanitize)\s*\(").unwrap(), SanitizerType::Html),
        ],
    };
}

lazy_static! {
    /// Go taint patterns.
    static ref GO_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // UserInput: fmt.Scan*, bufio.NewReader, bufio.NewScanner
            (Regex::new(r"(fmt\.Scan|bufio\.NewReader|bufio\.NewScanner)").unwrap(), TaintSourceType::UserInput),
            // HttpParam: r.FormValue, r.PostFormValue, r.URL.Query(), .Query()
            (Regex::new(r"(r\.(FormValue|PostFormValue|URL\.Query)\s*\(|\.Query\(\))").unwrap(), TaintSourceType::HttpParam),
            // HttpBody: r.Body, .ReadAll(r.Body)
            (Regex::new(r"(r\.Body|\.ReadAll\(r\.Body\))").unwrap(), TaintSourceType::HttpBody),
            // EnvVar: os.Getenv
            (Regex::new(r"os\.Getenv\s*\(").unwrap(), TaintSourceType::EnvVar),
            // Stdin: os.Stdin
            (Regex::new(r"os\.Stdin").unwrap(), TaintSourceType::Stdin),
            // FileRead: os.Open, ioutil.ReadFile
            (Regex::new(r"(os\.Open|ioutil\.ReadFile)\s*\(").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // ShellExec: exec.Command
            (Regex::new(r"exec\.Command\s*\(").unwrap(), TaintSinkType::ShellExec),
            // SqlQuery: db.Exec, db.Query, db.QueryRow
            (Regex::new(r"db\.(Exec|Query|QueryRow)\s*\(").unwrap(), TaintSinkType::SqlQuery),
            // FileWrite: template.HTML (XSS), fmt.Fprintf(w, ...)
            (Regex::new(r"(template\.HTML\s*\(|fmt\.Fprintf\s*\()").unwrap(), TaintSinkType::FileWrite),
        ],
        sanitizers: vec![
            // Numeric: strconv.Atoi, strconv.ParseInt, strconv.ParseFloat
            (Regex::new(r"strconv\.(Atoi|ParseInt|ParseFloat)\s*\(").unwrap(), SanitizerType::Numeric),
            // Html: html.EscapeString, url.QueryEscape
            (Regex::new(r"(html\.EscapeString|url\.QueryEscape)\s*\(").unwrap(), SanitizerType::Html),
        ],
    };
}

lazy_static! {
    /// Java taint patterns.
    static ref JAVA_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // Stdin: new Scanner(System.in)
            (Regex::new(r"new\s+Scanner\s*\(System\.in\)").unwrap(), TaintSourceType::Stdin),
            // UserInput: readLine(), new BufferedReader
            (Regex::new(r"(readLine\s*\(|new\s+BufferedReader\s*\()").unwrap(), TaintSourceType::UserInput),
            // HttpParam: request.getParameter, getQueryString
            (Regex::new(r"(request\.getParameter\s*\(|getQueryString\s*\()").unwrap(), TaintSourceType::HttpParam),
            // EnvVar: System.getenv
            (Regex::new(r"System\.getenv\s*\(").unwrap(), TaintSourceType::EnvVar),
            // FileRead: new FileReader, Files.readAllLines
            (Regex::new(r"(new\s+FileReader|Files\.readAllLines)").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // ShellExec: Runtime.getRuntime().exec, ProcessBuilder
            (Regex::new(r"(Runtime\.getRuntime\(\)\.exec\s*\(|ProcessBuilder\s*\()").unwrap(), TaintSinkType::ShellExec),
            // SqlQuery: Statement/Connection.execute/executeQuery/executeUpdate
            (Regex::new(r"\.(execute|executeQuery|executeUpdate)\s*\(").unwrap(), TaintSinkType::SqlQuery),
            // CodeEval: Class.forName
            (Regex::new(r"Class\.forName\s*\(").unwrap(), TaintSinkType::CodeEval),
        ],
        sanitizers: vec![
            // Numeric: Integer.parseInt, Long.parseLong, Double.parseDouble
            (Regex::new(r"(Integer\.parseInt|Long\.parseLong|Double\.parseDouble)\s*\(").unwrap(), SanitizerType::Numeric),
            // Html: ESAPI.encoder, StringEscapeUtils.escapeHtml
            (Regex::new(r"(ESAPI\.encoder\s*\(|StringEscapeUtils\.escapeHtml)").unwrap(), SanitizerType::Html),
        ],
    };
}

lazy_static! {
    /// Rust taint patterns.
    static ref RUST_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // Stdin: std::io::stdin(), io::stdin()
            (Regex::new(r"(std::)?io::stdin\s*\(").unwrap(), TaintSourceType::Stdin),
            // EnvVar: std::env::var, std::env::args, env::var, env::args
            (Regex::new(r"(std::)?env::var\s*\(").unwrap(), TaintSourceType::EnvVar),
            // UserInput: std::env::args (command line args are user input)
            (Regex::new(r"(std::)?env::args\s*\(").unwrap(), TaintSourceType::UserInput),
            // FileRead: std::fs::read_to_string, fs::read_to_string, File::open
            (Regex::new(r"((std::)?fs::read_to_string\s*\(|File::open)").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // ShellExec: Command::new, std::process::Command
            (Regex::new(r"(Command::new\s*\(|std::process::Command)").unwrap(), TaintSinkType::ShellExec),
            // CodeEval: unsafe block
            (Regex::new(r"\bunsafe\s*\{").unwrap(), TaintSinkType::CodeEval),
            // FileWrite: std::ptr::write, std::ptr::read
            (Regex::new(r"std::ptr::(write|read)\s*\(").unwrap(), TaintSinkType::FileWrite),
        ],
        sanitizers: vec![
            // Numeric: .parse::<i32>(), etc.
            (Regex::new(r"\.parse::<(i32|i64|u32|u64|f32|f64|usize|isize)>\s*\(").unwrap(), SanitizerType::Numeric),
        ],
    };
}

lazy_static! {
    /// C taint patterns.
    static ref C_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // UserInput: scanf, fscanf, sscanf
            (Regex::new(r"\b(scanf|fscanf|sscanf)\s*\(").unwrap(), TaintSourceType::UserInput),
            // UserInput: fgets, gets, getchar
            (Regex::new(r"\b(fgets|gets|getchar)\s*\(").unwrap(), TaintSourceType::UserInput),
            // EnvVar: getenv
            (Regex::new(r"\bgetenv\s*\(").unwrap(), TaintSourceType::EnvVar),
            // FileRead: fread, fopen
            (Regex::new(r"\b(fread|fopen)\s*\(").unwrap(), TaintSourceType::FileRead),
            // UserInput: recv, recvfrom (network)
            (Regex::new(r"\b(recv|recvfrom)\s*\(").unwrap(), TaintSourceType::UserInput),
        ],
        sinks: vec![
            // ShellExec: system, popen, execl, execv, execvp
            (Regex::new(r"\b(system|popen|execl|execv|execvp)\s*\(").unwrap(), TaintSinkType::ShellExec),
            // ShellExec: sprintf, vsprintf (format string vuln)
            (Regex::new(r"\b(sprintf|vsprintf)\s*\(").unwrap(), TaintSinkType::ShellExec),
            // FileWrite: strcpy, strcat, strncpy (buffer overflow)
            (Regex::new(r"\b(strcpy|strcat|strncpy)\s*\(").unwrap(), TaintSinkType::FileWrite),
        ],
        sanitizers: vec![
            // Numeric: atoi, atol, atof, strtol, strtoul, strtod
            (Regex::new(r"\b(atoi|atol|atof|strtol|strtoul|strtod)\s*\(").unwrap(), SanitizerType::Numeric),
            // Shell: snprintf (bounded write)
            (Regex::new(r"\bsnprintf\s*\(").unwrap(), SanitizerType::Shell),
        ],
    };
}

lazy_static! {
    /// C++ taint patterns.
    static ref CPP_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // UserInput: std::cin >>
            (Regex::new(r"std::cin\s*>>").unwrap(), TaintSourceType::UserInput),
            // UserInput: std::getline, getline
            (Regex::new(r"(std::)?getline\s*\(").unwrap(), TaintSourceType::UserInput),
            // EnvVar: getenv
            (Regex::new(r"\bgetenv\s*\(").unwrap(), TaintSourceType::EnvVar),
            // FileRead: std::ifstream, std::fstream
            (Regex::new(r"std::(ifstream|fstream)").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // ShellExec: system, popen, std::system
            (Regex::new(r"(\bsystem\s*\(|\bpopen\s*\(|std::system\s*\()").unwrap(), TaintSinkType::ShellExec),
            // ShellExec: sprintf
            (Regex::new(r"\bsprintf\s*\(").unwrap(), TaintSinkType::ShellExec),
        ],
        sanitizers: vec![
            // Numeric: std::stoi, std::stol, std::stoul, std::stoll, std::stof, std::stod
            (Regex::new(r"std::sto(i|l|ul|ll|f|d)\s*\(").unwrap(), SanitizerType::Numeric),
            // Numeric: static_cast<int/long/float/double>
            (Regex::new(r"static_cast<(int|long|float|double)>\s*\(").unwrap(), SanitizerType::Numeric),
        ],
    };
}

lazy_static! {
    /// Ruby taint patterns.
    static ref RUBY_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // UserInput: gets
            (Regex::new(r"\bgets\b").unwrap(), TaintSourceType::UserInput),
            // Stdin: STDIN.read, STDIN.gets, STDIN.readline
            (Regex::new(r"STDIN\.(read|gets|readline)").unwrap(), TaintSourceType::Stdin),
            // HttpParam: params[
            (Regex::new(r"\bparams\[").unwrap(), TaintSourceType::HttpParam),
            // EnvVar: ENV[
            (Regex::new(r"ENV\[").unwrap(), TaintSourceType::EnvVar),
            // FileRead: File.read, File.open
            (Regex::new(r"File\.(read|open)\s*\(").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // CodeEval: eval
            (Regex::new(r"\beval\s*\(").unwrap(), TaintSinkType::CodeEval),
            // ShellExec: system, exec
            (Regex::new(r"\b(system|exec)\s*\(").unwrap(), TaintSinkType::ShellExec),
            // ShellExec: IO.popen
            (Regex::new(r"IO\.popen\s*\(").unwrap(), TaintSinkType::ShellExec),
            // CodeEval: send (dynamic dispatch)
            (Regex::new(r"\.send\s*\(").unwrap(), TaintSinkType::CodeEval),
        ],
        sanitizers: vec![
            // Numeric: .to_i, .to_f
            (Regex::new(r"\.(to_i|to_f)\b").unwrap(), SanitizerType::Numeric),
            // Html: CGI.escapeHTML, Rack::Utils.escape_html
            (Regex::new(r"(CGI\.escapeHTML|Rack::Utils\.escape_html)\s*\(").unwrap(), SanitizerType::Html),
        ],
    };
}

lazy_static! {
    /// Kotlin taint patterns.
    static ref KOTLIN_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // UserInput: readLine(), readln()
            (Regex::new(r"\b(readLine|readln)\s*\(\)").unwrap(), TaintSourceType::UserInput),
            // EnvVar: System.getenv
            (Regex::new(r"System\.getenv\s*\(").unwrap(), TaintSourceType::EnvVar),
            // UserInput: BufferedReader
            (Regex::new(r"BufferedReader\s*\(").unwrap(), TaintSourceType::UserInput),
            // HttpParam: request.getParameter
            (Regex::new(r"request\.getParameter\s*\(").unwrap(), TaintSourceType::HttpParam),
        ],
        sinks: vec![
            // ShellExec: Runtime.getRuntime().exec, ProcessBuilder
            (Regex::new(r"(Runtime\.getRuntime\(\)\.exec\s*\(|ProcessBuilder\s*\()").unwrap(), TaintSinkType::ShellExec),
            // SqlQuery: .execute, .executeQuery, prepareStatement
            (Regex::new(r"\.(execute|executeQuery)\s*\(|prepareStatement\s*\(").unwrap(), TaintSinkType::SqlQuery),
        ],
        sanitizers: vec![
            // Numeric: .toInt(), .toLong(), .toDouble(), .toFloat()
            (Regex::new(r"\.(toInt|toLong|toDouble|toFloat)\s*\(\)").unwrap(), SanitizerType::Numeric),
        ],
    };
}

lazy_static! {
    /// Swift taint patterns.
    static ref SWIFT_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // UserInput: readLine()
            (Regex::new(r"\breadLine\s*\(\)").unwrap(), TaintSourceType::UserInput),
            // EnvVar: ProcessInfo.processInfo.environment[
            (Regex::new(r"ProcessInfo\.processInfo\.environment\[").unwrap(), TaintSourceType::EnvVar),
            // FileRead: FileManager.default, URLSession
            (Regex::new(r"(FileManager\.default|URLSession)").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // ShellExec: Process(), NSTask
            (Regex::new(r"(Process\s*\(\)|NSTask)").unwrap(), TaintSinkType::ShellExec),
            // SqlQuery: sqlite3_exec
            (Regex::new(r"sqlite3_exec\s*\(").unwrap(), TaintSinkType::SqlQuery),
        ],
        sanitizers: vec![
            // Numeric: Int(), Double(), Float()
            (Regex::new(r"\b(Int|Double|Float)\s*\(").unwrap(), SanitizerType::Numeric),
            // Html: addingPercentEncoding
            (Regex::new(r"addingPercentEncoding\s*\(").unwrap(), SanitizerType::Html),
        ],
    };
}

lazy_static! {
    /// C# taint patterns.
    static ref CSHARP_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // UserInput: Console.ReadLine()
            (Regex::new(r"Console\.ReadLine\s*\(").unwrap(), TaintSourceType::UserInput),
            // HttpParam: Request.QueryString[, Request.Form[
            (Regex::new(r"Request\.(QueryString|Form)\[").unwrap(), TaintSourceType::HttpParam),
            // EnvVar: Environment.GetEnvironmentVariable
            (Regex::new(r"Environment\.GetEnvironmentVariable\s*\(").unwrap(), TaintSourceType::EnvVar),
            // FileRead: File.ReadAllText, File.ReadAllLines, File.OpenRead, StreamReader
            (Regex::new(r"(File\.(ReadAllText|ReadAllLines|OpenRead)\s*\(|StreamReader\s*\()").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // ShellExec: Process.Start
            (Regex::new(r"Process\.Start\s*\(").unwrap(), TaintSinkType::ShellExec),
            // SqlQuery: SqlCommand, .ExecuteNonQuery, .ExecuteReader
            (Regex::new(r"(SqlCommand\s*\(|\.ExecuteNonQuery\s*\(|\.ExecuteReader\s*\()").unwrap(), TaintSinkType::SqlQuery),
            // CodeEval: Activator.CreateInstance
            (Regex::new(r"Activator\.CreateInstance\s*\(").unwrap(), TaintSinkType::CodeEval),
        ],
        sanitizers: vec![
            // Numeric: int.Parse, Convert.ToInt32, double.Parse
            (Regex::new(r"(int\.Parse|Convert\.ToInt32|double\.Parse)\s*\(").unwrap(), SanitizerType::Numeric),
            // Html: HttpUtility.HtmlEncode
            (Regex::new(r"HttpUtility\.HtmlEncode\s*\(").unwrap(), SanitizerType::Html),
        ],
    };
}

lazy_static! {
    /// Scala taint patterns.
    static ref SCALA_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // UserInput: StdIn.readLine, scala.io.StdIn
            (Regex::new(r"(StdIn\.readLine\s*\(|scala\.io\.StdIn)").unwrap(), TaintSourceType::UserInput),
            // EnvVar: System.getenv
            (Regex::new(r"System\.getenv\s*\(").unwrap(), TaintSourceType::EnvVar),
            // FileRead: Source.fromFile
            (Regex::new(r"Source\.fromFile\s*\(").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // ShellExec: Runtime.getRuntime.exec, sys.process, Process()
            (Regex::new(r"(Runtime\.getRuntime\.exec\s*\(|sys\.process|Process\s*\()").unwrap(), TaintSinkType::ShellExec),
            // SqlQuery: stmt.execute, statement.execute, .executeQuery
            (Regex::new(r"\.(execute|executeQuery)\s*\(").unwrap(), TaintSinkType::SqlQuery),
        ],
        sanitizers: vec![
            // Numeric: .toInt, .toLong, .toDouble
            (Regex::new(r"\.(toInt|toLong|toDouble)\b").unwrap(), SanitizerType::Numeric),
            // Html: StringEscapeUtils.escapeHtml
            (Regex::new(r"StringEscapeUtils\.escapeHtml").unwrap(), SanitizerType::Html),
        ],
    };
}

lazy_static! {
    /// PHP taint patterns.
    static ref PHP_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // HttpParam: $_GET, $_REQUEST, $_COOKIE, $_SERVER
            (Regex::new(r"\$_(GET|REQUEST|COOKIE|SERVER)\[").unwrap(), TaintSourceType::HttpParam),
            // HttpBody: $_POST
            (Regex::new(r"\$_POST\[").unwrap(), TaintSourceType::HttpBody),
            // UserInput: fgets
            (Regex::new(r"\bfgets\s*\(").unwrap(), TaintSourceType::UserInput),
            // FileRead: file_get_contents
            (Regex::new(r"file_get_contents\s*\(").unwrap(), TaintSourceType::FileRead),
            // EnvVar: getenv, $_ENV
            (Regex::new(r"(getenv\s*\(|\$_ENV\[)").unwrap(), TaintSourceType::EnvVar),
        ],
        sinks: vec![
            // CodeEval: eval
            (Regex::new(r"\beval\s*\(").unwrap(), TaintSinkType::CodeEval),
            // ShellExec: exec, system, passthru, shell_exec, popen, proc_open
            (Regex::new(r"\b(exec|system|passthru|shell_exec|popen|proc_open)\s*\(").unwrap(), TaintSinkType::ShellExec),
            // SqlQuery: mysqli_query, ->query
            (Regex::new(r"(mysqli_query\s*\(|->query\s*\()").unwrap(), TaintSinkType::SqlQuery),
        ],
        sanitizers: vec![
            // Numeric: intval, floatval, (int), (float)
            (Regex::new(r"(\b(intval|floatval)\s*\(|\(int\)|\(float\))").unwrap(), SanitizerType::Numeric),
            // Html: htmlspecialchars, htmlentities
            (Regex::new(r"(htmlspecialchars|htmlentities)\s*\(").unwrap(), SanitizerType::Html),
            // Shell: mysqli_real_escape_string
            (Regex::new(r"mysqli_real_escape_string\s*\(").unwrap(), SanitizerType::Shell),
        ],
    };
}

lazy_static! {
    /// Lua/Luau taint patterns.
    static ref LUA_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // UserInput: io.read
            (Regex::new(r"io\.read\s*\(").unwrap(), TaintSourceType::UserInput),
            // EnvVar: os.getenv
            (Regex::new(r"os\.getenv\s*\(").unwrap(), TaintSourceType::EnvVar),
            // FileRead: io.open
            (Regex::new(r"io\.open\s*\(").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // ShellExec: os.execute
            (Regex::new(r"os\.execute\s*\(").unwrap(), TaintSinkType::ShellExec),
            // ShellExec: io.popen
            (Regex::new(r"io\.popen\s*\(").unwrap(), TaintSinkType::ShellExec),
            // CodeEval: loadstring, load, dofile, loadfile
            (Regex::new(r"\b(loadstring|load|dofile|loadfile)\s*\(").unwrap(), TaintSinkType::CodeEval),
        ],
        sanitizers: vec![
            // Numeric: tonumber
            (Regex::new(r"\btonumber\s*\(").unwrap(), SanitizerType::Numeric),
        ],
    };
}

lazy_static! {
    /// Elixir taint patterns.
    static ref ELIXIR_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // UserInput: IO.gets
            (Regex::new(r"IO\.gets\s*\(").unwrap(), TaintSourceType::UserInput),
            // EnvVar: System.get_env
            (Regex::new(r"System\.get_env\s*\(").unwrap(), TaintSourceType::EnvVar),
            // FileRead: File.read, File.read!
            (Regex::new(r"File\.(read|read!)\s*\(").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // ShellExec: System.cmd
            (Regex::new(r"System\.cmd\s*\(").unwrap(), TaintSinkType::ShellExec),
            // CodeEval: Code.eval_string
            (Regex::new(r"Code\.eval_string\s*\(").unwrap(), TaintSinkType::CodeEval),
            // SqlQuery: Ecto.Adapters.SQL.query
            (Regex::new(r"Ecto\.Adapters\.SQL\.query\s*\(").unwrap(), TaintSinkType::SqlQuery),
        ],
        sanitizers: vec![
            // Numeric: String.to_integer, String.to_float
            (Regex::new(r"String\.(to_integer|to_float)\s*\(").unwrap(), SanitizerType::Numeric),
            // Html: Phoenix.HTML.html_escape
            (Regex::new(r"Phoenix\.HTML\.html_escape\s*\(").unwrap(), SanitizerType::Html),
        ],
    };
}

lazy_static! {
    /// OCaml taint patterns.
    static ref OCAML_PATTERNS: LanguagePatterns = LanguagePatterns {
        sources: vec![
            // UserInput: read_line
            (Regex::new(r"\bread_line\s").unwrap(), TaintSourceType::UserInput),
            // EnvVar: Sys.getenv
            (Regex::new(r"Sys\.getenv\s").unwrap(), TaintSourceType::EnvVar),
            // UserInput: input_line (reading from a channel)
            (Regex::new(r"\binput_line\s").unwrap(), TaintSourceType::UserInput),
            // FileRead: In_channel.read_all, In_channel.input_all
            (Regex::new(r"In_channel\.(read_all|input_all)\s").unwrap(), TaintSourceType::FileRead),
        ],
        sinks: vec![
            // ShellExec: Sys.command
            (Regex::new(r"Sys\.command\s").unwrap(), TaintSinkType::ShellExec),
            // ShellExec: Unix.execvp
            (Regex::new(r"Unix\.execvp\s").unwrap(), TaintSinkType::ShellExec),
            // SqlQuery: Sqlite3.exec
            (Regex::new(r"Sqlite3\.exec\s").unwrap(), TaintSinkType::SqlQuery),
        ],
        sanitizers: vec![
            // Numeric: int_of_string, float_of_string
            (Regex::new(r"\b(int_of_string|float_of_string)\s").unwrap(), SanitizerType::Numeric),
        ],
    };
}

/// Get taint analysis patterns for a given language.
///
/// Each language has its own set of source, sink, and sanitizer patterns.
/// TypeScript/JavaScript share patterns, as do Lua/Luau.
pub fn get_patterns(language: Language) -> &'static LanguagePatterns {
    match language {
        Language::Python => &PYTHON_PATTERNS,
        Language::TypeScript | Language::JavaScript => &TYPESCRIPT_PATTERNS,
        Language::Go => &GO_PATTERNS,
        Language::Java => &JAVA_PATTERNS,
        Language::Rust => &RUST_PATTERNS,
        Language::C => &C_PATTERNS,
        Language::Cpp => &CPP_PATTERNS,
        Language::Ruby => &RUBY_PATTERNS,
        Language::Kotlin => &KOTLIN_PATTERNS,
        Language::Swift => &SWIFT_PATTERNS,
        Language::CSharp => &CSHARP_PATTERNS,
        Language::Scala => &SCALA_PATTERNS,
        Language::Php => &PHP_PATTERNS,
        Language::Lua | Language::Luau => &LUA_PATTERNS,
        Language::Elixir => &ELIXIR_PATTERNS,
        Language::Ocaml => &OCAML_PATTERNS,
    }
}

/// Detect taint sources in a statement.
///
/// Scans the statement for patterns matching known taint sources (e.g., `input()`,
/// `request.args`, `os.environ`). If a source is found and the statement is an
/// assignment, returns a `TaintSource` with the assigned variable name.
///
/// # Arguments
///
/// * `statement` - The source code statement to analyze
/// * `line` - The line number of the statement
/// * `language` - The programming language (determines which patterns to use)
///
/// # Returns
///
/// A vector of detected `TaintSource`s. Usually 0 or 1, but could be more if
/// multiple sources appear in the same statement.
pub fn detect_sources(statement: &str, line: u32, language: Language) -> Vec<TaintSource> {
    let mut sources = Vec::new();
    let patterns = get_patterns(language);

    for (pattern, source_type) in patterns.sources.iter() {
        if pattern.is_match(statement) {
            // Try to extract variable name from assignment (left side of =)
            if let Some(var) = extract_assigned_var(statement) {
                sources.push(TaintSource {
                    var,
                    line,
                    source_type: *source_type,
                    statement: Some(statement.to_string()),
                });
            } else {
                // For non-assignment sources (e.g., C's scanf(buf), fgets(buf, ...)),
                // extract the first variable argument from the call
                if let Some(var) = extract_call_arg(statement, pattern) {
                    sources.push(TaintSource {
                        var,
                        line,
                        source_type: *source_type,
                        statement: Some(statement.to_string()),
                    });
                } else {
                    // Last resort: use a synthetic variable name from the source type
                    // This handles patterns like "std::cin >> input" or "STDIN.read"
                    // where neither assignment nor call extraction works
                    let var = extract_source_var_from_statement(statement);
                    if let Some(var) = var {
                        sources.push(TaintSource {
                            var,
                            line,
                            source_type: *source_type,
                            statement: Some(statement.to_string()),
                        });
                    }
                }
            }
        }
    }

    sources
}

/// Extract a variable name from a source statement when there's no assignment or call arg.
///
/// Handles patterns like:
/// - "std::cin >> input" -> "input"
/// - "fmt.Scan(&input)" -> "input"
/// - "std::ifstream file(path)" -> "file"
/// - "scanf(\"%s\", buf)" -> "buf" (already handled by extract_call_arg)
fn extract_source_var_from_statement(statement: &str) -> Option<String> {
    // Handle C++ "cin >> var" pattern
    if let Some(pos) = statement.find(">>") {
        let after = statement[pos + 2..].trim();
        let var = after.split_whitespace().next().unwrap_or("");
        let var = var.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if is_valid_identifier(var) {
            return Some(var.to_string());
        }
    }

    // Handle "&var" references (Go's fmt.Scan(&input))
    if let Some(pos) = statement.find('&') {
        let after = &statement[pos + 1..];
        let var = after
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        if is_valid_identifier(var) {
            return Some(var.to_string());
        }
    }

    // Handle C++ constructor-style declarations: "Type var(args)" or "Type var"
    // e.g., "std::ifstream file(path)" -> "file"
    let tokens: Vec<&str> = statement.split_whitespace().collect();
    if tokens.len() >= 2 {
        // Find a token that looks like a variable (followed by '(' or end)
        for tok in tokens.iter().skip(1) {
            // Strip trailing '(' and everything after for constructor calls
            let var = tok.split('(').next().unwrap_or(tok);
            let var = var.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if is_valid_identifier(var) && var.len() > 1 {
                return Some(var.to_string());
            }
        }
    }

    None
}

/// Detect taint sinks in a statement.
///
/// Scans the statement for patterns matching known taint sinks (e.g., `execute()`,
/// `eval()`, `os.system()`). If a sink is found, extracts the variable being
/// passed as an argument.
///
/// # Arguments
///
/// * `statement` - The source code statement to analyze
/// * `line` - The line number of the statement
/// * `language` - The programming language (determines which patterns to use)
///
/// # Returns
///
/// A vector of detected `TaintSink`s. The `tainted` field is set to `false`
/// initially; it will be updated by the taint propagation analysis.
pub fn detect_sinks(statement: &str, line: u32, language: Language) -> Vec<TaintSink> {
    let mut sinks = Vec::new();
    let patterns = get_patterns(language);
    for (pattern, sink_type) in patterns.sinks.iter() {
        if pattern.is_match(statement) {
            // Extract variable name from call argument
            if let Some(var) = extract_call_arg(statement, pattern) {
                sinks.push(TaintSink {
                    var,
                    line,
                    sink_type: *sink_type,
                    tainted: false,
                    statement: Some(statement.to_string()),
                });
            } else {
                // Handle assignment-style sinks (e.g., "element.innerHTML = userContent")
                // and patterns where the dangerous argument is on the RHS of an assignment
                if let Some(var) = extract_sink_var_from_statement(statement, pattern) {
                    sinks.push(TaintSink {
                        var,
                        line,
                        sink_type: *sink_type,
                        tainted: false,
                        statement: Some(statement.to_string()),
                    });
                } else {
                    // Fallback: extract interpolated variables from format strings.
                    // This catches f"SELECT {query}", `SELECT ${query}`, etc.
                    // where the sink argument is a string literal with embedded variables.
                    let interp_vars = extract_interpolated_vars(statement);
                    for var in interp_vars {
                        sinks.push(TaintSink {
                            var,
                            line,
                            sink_type: *sink_type,
                            tainted: false,
                            statement: Some(statement.to_string()),
                        });
                }
            }
        }
    }
}
sinks
}

/// Extract a variable from a sink statement when extract_call_arg fails.
///
/// Handles:
/// - Assignment-style sinks: "element.innerHTML = userContent" -> "userContent"
/// - Non-call sinks: "unsafe { std::ptr::write(ptr, val) }" -> "ptr"
/// - Process constructors: "new ProcessBuilder(cmd).start()" -> "cmd"
/// - Space-separated args (OCaml): "Unix.execvp cmd args" -> "cmd"
/// - Scala: "import sys.process._; cmd.!" -> "cmd"
fn extract_sink_var_from_statement(statement: &str, pattern: &Regex) -> Option<String> {
    if let Some(m) = pattern.find(statement) {
        let after = &statement[m.end()..];
        let after = after.trim();

        // If the pattern matched an assignment (innerHTML =), RHS is the var
        if after.is_empty() || !after.starts_with('(') {
            // Get what's after the "=" in the full statement
            if let Some(eq_pos) = statement.rfind('=') {
                // Make sure it's not == or other compound operators
                let before_eq = if eq_pos > 0 {
                    statement.as_bytes()[eq_pos - 1]
                } else {
                    b' '
                };
                let after_eq = if eq_pos + 1 < statement.len() {
                    statement.as_bytes()[eq_pos + 1]
                } else {
                    b' '
                };
                if before_eq != b'='
                    && before_eq != b'!'
                    && before_eq != b'<'
                    && before_eq != b'>'
                    && after_eq != b'='
                {
                    let rhs = statement[eq_pos + 1..].trim();
                    let var = rhs
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .next()
                        .unwrap_or("");
                    if is_valid_identifier(var) {
                        return Some(var.to_string());
                    }
                }
            }
        }

        // Try to find a parenthesized argument after the pattern
        // Handle "new ProcessBuilder(cmd).start()" or "ProcessBuilder(cmd)"
        let search_area = &statement[m.start()..];
        if let Some(open) = search_area.find('(') {
            let rest = &search_area[open + 1..];
            let end = rest.find([',', ')']).unwrap_or(rest.len());
            let arg = rest[..end].trim();
            if !arg.starts_with('"') && !arg.starts_with('\'') && !arg.is_empty() {
                let var_name = arg.split('.').next().unwrap_or(arg);
                let var_name = var_name.trim_start_matches('$');
                if is_valid_identifier(var_name) {
                    return Some(var_name.to_string());
                }
            }
        }

        // Handle space-separated arguments (OCaml, Haskell, etc.)
        // e.g., "Unix.execvp cmd args" -> "cmd"
        // e.g., "Sys.command cmd" -> "cmd"
        if !after.is_empty() && !after.starts_with('(') {
            // Take the first space-separated token after the pattern
            let token = after
                .split(|c: char| c.is_whitespace() || c == ';')
                .next()
                .unwrap_or("");
            let token = token.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if is_valid_identifier(token) {
                return Some(token.to_string());
            }
        }

        // Handle semicolon-separated statements
        // e.g., "import sys.process._; cmd.!" -> look before semicolon for var
        if statement.contains(';') {
            // Look for identifiers in the other parts of the statement
            for part in statement.split(';') {
                let part = part.trim();
                // Skip the part that contains the pattern match
                if pattern.is_match(part) {
                    continue;
                }
                // Find first identifier in this part
                let var = part
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .find(|t| is_valid_identifier(t));
                if let Some(var) = var {
                    return Some(var.to_string());
                }
            }
        }
    }

    None
}

/// Check if a statement contains a sanitizer and return its type.
///
/// Scans for patterns like `int()`, `shlex.quote()`, `html.escape()` that
/// neutralize taint for specific sink types.
///
/// # Arguments
///
/// * `statement` - The source code statement to analyze
/// * `language` - The programming language (determines which patterns to use)
///
/// # Returns
///
/// `Some(SanitizerType)` if a sanitizer is detected, `None` otherwise.
pub fn detect_sanitizer(statement: &str, language: Language) -> Option<SanitizerType> {
    let patterns = get_patterns(language);
    for (pattern, sanitizer_type) in patterns.sanitizers.iter() {
        if pattern.is_match(statement) {
            return Some(*sanitizer_type);
        }
    }
    None
}

/// Convenience wrapper to check if a statement contains any sanitizer.
///
/// # Arguments
///
/// * `statement` - The source code statement to analyze
/// * `language` - The programming language (determines which patterns to use)
///
/// # Returns
///
/// `true` if the statement contains a sanitizer, `false` otherwise.
pub fn is_sanitizer(statement: &str, language: Language) -> bool {
    detect_sanitizer(statement, language).is_some()
}

/// Find sanitizers in a statement and return the sanitized variable with its type.
///
/// # Arguments
///
/// * `statement` - The source code statement to analyze
/// * `line` - The line number of the statement
/// * `language` - The programming language (determines which patterns to use)
///
/// # Returns
///
/// A vector of (variable_name, SanitizerType) pairs for each sanitizer found.
pub fn find_sanitizers_in_statement(
    statement: &str,
    _line: u32,
    language: Language,
) -> Vec<(String, SanitizerType)> {
    let mut result = Vec::new();
    let patterns = get_patterns(language);

    for (pattern, sanitizer_type) in patterns.sanitizers.iter() {
        if pattern.is_match(statement) {
            // The sanitized variable is the one being assigned to
            if let Some(var) = extract_assigned_var(statement) {
                result.push((var, *sanitizer_type));
            }
        }
    }

    result
}

/// Extract variable name from assignment (LHS of =).
///
/// Handles various Python assignment patterns:
/// - Simple assignment: `var = ...`
/// - Type-annotated assignment: `var: Type = ...`
/// - Walrus operator: `(var := ...)`
///
/// # Arguments
///
/// * `statement` - The source code statement to analyze
///
/// # Returns
///
/// `Some(variable_name)` if an assignment is detected, `None` otherwise.
fn extract_assigned_var(statement: &str) -> Option<String> {
    let trimmed = statement.trim();

    // Walrus operator / Go short declaration: (var := ...) or var := ...
    if let Some(pos) = trimmed.find(":=") {
        let before = &trimmed[..pos];
        let var = before.trim().trim_start_matches('(').trim();
        if is_valid_identifier(var) {
            return Some(var.to_string());
        }
        // Handle "rows, err := ..." -> take first variable
        if let Some(first) = var.split(',').next() {
            let first = first.trim();
            if is_valid_identifier(first) {
                return Some(first.to_string());
            }
        }
    }

    // Standard assignment: var = ...
    if let Some(pos) = trimmed.find('=') {
        // Skip == comparison
        if pos > 0 && trimmed.chars().nth(pos.saturating_sub(1)) == Some('=') {
            return None;
        }
        if pos + 1 < trimmed.len() && trimmed.chars().nth(pos + 1) == Some('=') {
            return None;
        }
        // Skip !=, <=, >=
        if pos > 0 {
            let prev_char = trimmed.chars().nth(pos.saturating_sub(1));
            if prev_char == Some('!') || prev_char == Some('<') || prev_char == Some('>') {
                return None;
            }
        }

        let before = &trimmed[..pos];
        // Handle type annotation: var: Type = ...
        let var_part = if let Some(colon_pos) = before.find(':') {
            &before[..colon_pos]
        } else {
            before
        };
        let var = var_part.trim();
        if is_valid_identifier(var) {
            return Some(var.to_string());
        }

        // Handle multi-language patterns where there are keywords/types before the var:
        // JavaScript/TypeScript: const/let/var name = ...
        // Rust: let/let mut name = ...
        // Java/C#: TypeName name = ...
        // C/C++: type *name = ..., type name = ...
        // Kotlin: val/var name = ...
        // Swift: let/var name = ...
        // Lua: local name = ...
        // Scala: val/var name = ...
        // OCaml: let name = ...
        // PHP: $name = ...
        let tokens: Vec<&str> = var.split_whitespace().collect();
        if tokens.len() >= 2 {
            // Take the last token as the variable name
            let last = tokens[tokens.len() - 1];
            // Strip pointer/reference markers for C/C++
            let clean = last.trim_start_matches('*').trim_start_matches('&');
            // Strip PHP $ prefix for validation but keep it
            let check = clean.trim_start_matches('$');
            if !check.is_empty() && is_valid_identifier(check) {
                return Some(clean.to_string());
            }
        }

        // Handle Elixir pattern match: {:ok, content} = ...
        // Extract last identifier from destructuring
        if var.contains('{') || var.contains('(') || var.contains('[') {
            // Find identifiers in the pattern
            let cleaned = var.replace(['{', '}', '(', ')', '[', ']', ':'], " ");
            let idents: Vec<&str> = cleaned
                .split_whitespace()
                .filter(|t| is_valid_identifier(t) && *t != "ok" && *t != "err")
                .collect();
            if let Some(last_ident) = idents.last() {
                return Some(last_ident.to_string());
            }
        }

        // Handle PHP $var = ... (single token starting with $)
        if let Some(name) = var.strip_prefix('$') {
            if is_valid_identifier(name) {
                return Some(var.to_string());
            }
        }
    }

    // No assignment found - check for function-call-as-source patterns
    // like scanf("%s", buf) or fgets(buf, ...) where the target variable
    // is an argument rather than the LHS of an assignment.
    // These are handled by detect_sources_from_call_args (separate path).
    None
}

/// Extract the first argument from a function call that matches the pattern.
///
/// # Arguments
///
/// * `statement` - The source code statement to analyze
/// * `pattern` - The regex pattern that matched (to find the right call)
///
/// # Returns
///
/// `Some(argument_name)` if a variable argument is found, `None` if the argument
/// is a string literal or not a valid identifier.
fn extract_call_arg(statement: &str, pattern: &Regex) -> Option<String> {
    // Find where the pattern matches, then find the opening paren
    if let Some(m) = pattern.find(statement) {
        let after_match = &statement[m.end()..];
        // The pattern includes the `(`, so we're already past it
        // But some patterns end with `\(`, so we need to handle that
        let rest = after_match.strip_prefix('(').unwrap_or(after_match);
        // Try each argument until we find a valid variable
        let mut remaining = rest;
        loop {
            // Find end of current argument (comma or close paren)
            let end = remaining
                .find([',', ')'])
                .unwrap_or(remaining.len());
            let arg = remaining[..end].trim();
            // Check if it's a variable (not a string literal)
            if !arg.is_empty()
                && !arg.starts_with('"')
                && !arg.starts_with('\'')
                && !arg.starts_with("f\"")
                && !arg.starts_with("f'")
                && !arg.starts_with("r\"")
                && !arg.starts_with("r'")
            {
                // Handle attribute access like obj.attr - just get the first part
                let var_name = arg.split('.').next().unwrap_or(arg);
                // Strip PHP $ prefix for validation
                let check_name = var_name.trim_start_matches('$');
                if is_valid_identifier(check_name) {
                    return Some(var_name.to_string());
                }
            }
            // String concatenation: "..." + var — extract var from RHS of +
            if arg.contains('+') {
                for part in arg.split('+') {
                    let part = part.trim();
                    if !part.is_empty()
                        && !part.starts_with('"')
                        && !part.starts_with('\'')
                        && !part.starts_with("f\"")
                        && !part.starts_with("f'")
                    {
                        let var_name = part.split('.').next().unwrap_or(part);
                        let check_name = var_name.trim_start_matches('$');
                        if is_valid_identifier(check_name) {
                            return Some(var_name.to_string());
            }
            }
        }
    }
    // Move to next argument
    if end >= remaining.len() {
        break;
}
let next_char = remaining.as_bytes()[end];
if next_char == b')' {
    break;
}
// Skip comma and move to next arg
remaining = &remaining[end + 1..];
}
}
None
}

/// Extract interpolated variables from format strings (f-strings, template literals).
///
/// Handles:
/// - Python f-strings: `f"SELECT {query}"` -> ["query"]
/// - Python .format(): `"SELECT {}".format(query)` -> ["query"]
/// - Python % formatting: `"SELECT %s" % query` -> ["query"]
/// - JS/TS template literals: `` `SELECT ${query}` `` -> ["query"]
/// - Ruby interpolation: `"SELECT #{query}"` -> ["query"]
/// - Rust format!: `format!("SELECT {}", query)` -> ["query"]
///
/// Returns all valid identifier names found inside interpolation braces.
fn extract_interpolated_vars(statement: &str) -> Vec<String> {
    let mut vars = Vec::new();

    // Python f-string / JS template literal / Ruby: {var} or ${var} or #{var}
    // Match {identifier}, ${identifier}, #{identifier} patterns
    let _chars = statement.chars().peekable();
    let mut i = 0;
    let bytes = statement.as_bytes();

    while i < bytes.len() {
        // Detect interpolation start: { or ${ or #{
        let is_interp = match bytes[i] {
            b'{' => {
                // Could be f-string {var} or standalone — check it's not {{
                i + 1 < bytes.len() && bytes[i + 1] != b'{'
            }
            b'$' | b'#' => {
                // ${var} or #{var}
                i + 1 < bytes.len() && bytes[i + 1] == b'{'
            }
            _ => false,
        };

        if is_interp {
            // Skip to the opening brace
            let brace_start = if bytes[i] == b'{' { i } else { i + 1 };
            if brace_start + 1 < bytes.len() {
                // Find closing brace
                if let Some(close) = statement[brace_start + 1..].find('}') {
                    let inner = &statement[brace_start + 1..brace_start + 1 + close];
                    let inner = inner.trim();
                    // Could be an expression like `query` or `user.name` — take first identifier
                    let var_name = inner
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .next()
                        .unwrap_or("");
                    if is_valid_identifier(var_name) {
                        vars.push(var_name.to_string());
                    }
                    i = brace_start + 1 + close + 1;
                    continue;
                }
            }
        }

        // Python .format() args: "...".format(var1, var2)
        if i + 8 < bytes.len() && &statement[i..i + 8] == ".format(" {
            let args_start = i + 8;
            if let Some(close) = statement[args_start..].find(')') {
                let args_str = &statement[args_start..args_start + close];
                for arg in args_str.split(',') {
                    let arg = arg.trim();
                    // Skip keyword args like key=val, take val
                    let val = if let Some(eq_pos) = arg.find('=') {
                        arg[eq_pos + 1..].trim()
                    } else {
                        arg
                    };
                    let var_name = val
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .next()
                        .unwrap_or("");
                    if is_valid_identifier(var_name) {
                        vars.push(var_name.to_string());
                    }
                }
                i = args_start + close + 1;
                continue;
            }
        }

        // Python % formatting: "..." % (var,) or "..." % var
        if bytes[i] == b'%' && i > 0 {
            let before = statement[..i].trim_end();
            let after = statement[i + 1..].trim_start();
            if (before.ends_with('"') || before.ends_with('\'')) && !after.starts_with('%') {
                // Single var: "..." % var
                // Tuple: "..." % (var1, var2)
                let args_str = if after.starts_with('(') {
                    if let Some(close) = after.find(')') {
                        &after[1..close]
                    } else {
                        ""
                    }
                } else {
                    // Single variable
                    after.split(|c: char| c.is_whitespace() || c == ')' || c == ',')
                        .next()
                        .unwrap_or("")
                };
                for arg in args_str.split(',') {
                    let arg = arg.trim();
                    let var_name = arg
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .next()
                        .unwrap_or("");
                    if is_valid_identifier(var_name) {
                        vars.push(var_name.to_string());
                    }
                }
            }
        }

        i += 1;
    }

    // Deduplicate
    vars.sort();
    vars.dedup();
    vars
}

/// Check if a string is a valid Python identifier.
///
/// A valid identifier starts with a letter or underscore, and contains
/// only letters, digits, and underscores.
fn is_valid_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .map(|c| c.is_alphabetic() || c == '_')
            .unwrap_or(false)
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// Check if an identifier appears as a standalone word in text.
/// Uses word-boundary logic: the identifier must be surrounded by
/// non-alphanumeric, non-underscore characters (or be at string edges).
/// Prevents substring matches (e.g., "user" won't match inside "user_name").
fn identifier_in_text(text: &str, ident: &str) -> bool {
    let bytes = text.as_bytes();
    let ident_len = ident.len();
    if ident_len == 0 || ident_len > bytes.len() {
        return false;
    }
    let mut pos = 0;
    while pos + ident_len <= bytes.len() {
        match text[pos..].find(ident) {
            Some(offset) => {
                let abs = pos + offset;
                let before_ok = abs == 0 || {
                    let c = bytes[abs - 1];
                    !c.is_ascii_alphanumeric() && c != b'_'
                };
                let after_pos = abs + ident_len;
                let after_ok = after_pos >= bytes.len() || {
                    let c = bytes[after_pos];
                    !c.is_ascii_alphanumeric() && c != b'_'
                };
                if before_ok && after_ok {
                    return true;
                }
                pos = abs + 1;
            }
            None => break,
        }
    }
    false
}

/// Check if a statement contains only a constant string (no taint).
///
/// Used to reduce false positives - string literals are not tainted.
///
/// # Arguments
///
/// * `statement` - The source code statement to analyze
///
/// # Returns
///
/// `true` if the statement is a constant string assignment, `false` otherwise.
#[allow(dead_code)]
pub fn is_constant_string(statement: &str) -> bool {
    // Match patterns like: var = "string" or var = 'string'
    lazy_static! {
        static ref CONST_STRING: Regex = Regex::new(r#"^\s*\w+\s*=\s*["'][^"']*["']\s*$"#).unwrap();
    }
    CONST_STRING.is_match(statement)
}

/// Check if a statement uses ORM-safe patterns (parameterized queries).
///
/// SQLAlchemy and similar ORMs use operator overloading for safe queries.
/// These should not be flagged as SQL injection sinks.
///
/// # Arguments
///
/// * `statement` - The source code statement to analyze
///
/// # Returns
///
/// `true` if the statement uses ORM-safe patterns, `false` otherwise.
#[allow(dead_code)]
pub fn is_orm_safe_pattern(statement: &str) -> bool {
    lazy_static! {
        // SQLAlchemy patterns: session.query(...).filter(...), select(...).where(...)
        static ref ORM_SAFE: Regex =
            Regex::new(r"(\.filter\s*\(|\.where\s*\(|\.filter_by\s*\()").unwrap();
    }
    ORM_SAFE.is_match(statement)
}

// Aliases for test compatibility (tests use different naming)
pub use detect_sinks as find_sinks_in_statement;
pub use detect_sources as find_sources_in_statement;

// =============================================================================
// AST-Based Detection - Phase 9
// =============================================================================
//
// These functions use tree-sitter AST nodes to detect sources, sinks, and
// sanitizers. They complement the regex-based detection by:
// 1. Filtering out false positives from comments and string literals
// 2. Using structural matching instead of text patterns
// 3. Working with the full parsed tree for context
//
// The AST-based functions are used by `compute_taint_with_tree` and fall back
// to regex-based detection when the AST yields no results.

use super::ast_utils::{
    call_node_kinds, extract_call_name, find_parent_assignment_var, is_in_comment, is_in_string,
    node_text, walk_descendants,
};

/// AST-based source pattern: matches call names and member access patterns.
struct AstSourcePattern {
    /// Simple function names that indicate a source (e.g., "input", "readLine")
    call_names: &'static [&'static str],
    /// Dotted member access patterns (e.g., "request.args", "os.environ")
    /// Matched as substrings of the full call name text.
    member_patterns: &'static [&'static str],
    /// The source type to assign when matched
    source_type: TaintSourceType,
}

/// AST-based sink pattern.
struct AstSinkPattern {
    call_names: &'static [&'static str],
    member_patterns: &'static [&'static str],
    sink_type: TaintSinkType,
}

/// AST-based sanitizer pattern.
struct AstSanitizerPattern {
    call_names: &'static [&'static str],
    member_patterns: &'static [&'static str],
    sanitizer_type: SanitizerType,
}

/// Complete AST pattern set for a language.
struct AstLanguagePatterns {
    sources: &'static [AstSourcePattern],
    sinks: &'static [AstSinkPattern],
    sanitizers: &'static [AstSanitizerPattern],
}

// ---------------------------------------------------------------------------
// AST Pattern Definitions for All 18 Languages
// ---------------------------------------------------------------------------

static PYTHON_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &["input"],
        member_patterns: &[],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &[
            "request.args",
            "request.form",
            "request.values",
            "request.cookies",
            "request.headers",
        ],
        source_type: TaintSourceType::HttpParam,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["request.json", "request.data"],
        source_type: TaintSourceType::HttpParam,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["request.get_json"],
        source_type: TaintSourceType::HttpBody,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["sys.stdin"],
        source_type: TaintSourceType::Stdin,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["os.environ", "os.getenv"],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &[".read(", ".readlines(", ".readline("],
        source_type: TaintSourceType::FileRead,
    },
];

static PYTHON_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &[],
        member_patterns: &[".execute(", ".executemany("],
        sink_type: TaintSinkType::SqlQuery,
    },
    AstSinkPattern {
        call_names: &["eval"],
        member_patterns: &[],
        sink_type: TaintSinkType::CodeEval,
    },
    AstSinkPattern {
        call_names: &["exec"],
        member_patterns: &[],
        sink_type: TaintSinkType::CodeExec,
    },
    AstSinkPattern {
        call_names: &["compile"],
        member_patterns: &[],
        sink_type: TaintSinkType::CodeCompile,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &[
            "subprocess.run",
            "subprocess.call",
            "subprocess.Popen",
            "subprocess.check_output",
        ],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["os.system", "os.popen"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &[".write("],
        sink_type: TaintSinkType::FileWrite,
    },
];

static PYTHON_AST_SANITIZERS: &[AstSanitizerPattern] = &[
    AstSanitizerPattern {
        call_names: &["int", "float", "bool"],
        member_patterns: &[],
        sanitizer_type: SanitizerType::Numeric,
    },
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["shlex.quote", "pipes.quote"],
        sanitizer_type: SanitizerType::Shell,
    },
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["html.escape", "markupsafe.escape", "cgi.escape"],
        sanitizer_type: SanitizerType::Html,
    },
];

static TYPESCRIPT_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["req.body"],
        source_type: TaintSourceType::HttpBody,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["req.params", "req.query", "req.cookies", "req.headers"],
        source_type: TaintSourceType::HttpParam,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["process.env"],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["process.stdin"],
        source_type: TaintSourceType::Stdin,
    },
    AstSourcePattern {
        call_names: &["readline"],
        member_patterns: &[],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &[".read(", ".readFile("],
        source_type: TaintSourceType::FileRead,
    },
];

static TYPESCRIPT_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &["eval"],
        member_patterns: &[],
        sink_type: TaintSinkType::CodeEval,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["new Function"],
        sink_type: TaintSinkType::CodeEval,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &[
            "child_process.exec",
            "child_process.spawn",
            "child_process.execSync",
            "child_process.execFile",
        ],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &["execSync"],
        member_patterns: &[],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &[".innerHTML"],
        sink_type: TaintSinkType::FileWrite,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["document.write"],
        sink_type: TaintSinkType::FileWrite,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &[".query(", ".execute("],
        sink_type: TaintSinkType::SqlQuery,
    },
];

static TYPESCRIPT_AST_SANITIZERS: &[AstSanitizerPattern] = &[
    AstSanitizerPattern {
        call_names: &["parseInt", "Number", "parseFloat"],
        member_patterns: &[],
        sanitizer_type: SanitizerType::Numeric,
    },
    AstSanitizerPattern {
        call_names: &["encodeURIComponent"],
        member_patterns: &["DOMPurify.sanitize"],
        sanitizer_type: SanitizerType::Html,
    },
];

static GO_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["fmt.Scan", "bufio.NewReader", "bufio.NewScanner"],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["r.FormValue", "r.PostFormValue", "r.URL.Query", ".Query()"],
        source_type: TaintSourceType::HttpParam,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["r.Body", ".ReadAll(r.Body)"],
        source_type: TaintSourceType::HttpBody,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["os.Getenv"],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["os.Stdin"],
        source_type: TaintSourceType::Stdin,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["os.Open", "ioutil.ReadFile"],
        source_type: TaintSourceType::FileRead,
    },
];

static GO_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["exec.Command"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["db.Exec", "db.Query", "db.QueryRow"],
        sink_type: TaintSinkType::SqlQuery,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["template.HTML", "fmt.Fprintf"],
        sink_type: TaintSinkType::FileWrite,
    },
];

static GO_AST_SANITIZERS: &[AstSanitizerPattern] = &[
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["strconv.Atoi", "strconv.ParseInt", "strconv.ParseFloat"],
        sanitizer_type: SanitizerType::Numeric,
    },
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["html.EscapeString", "url.QueryEscape"],
        sanitizer_type: SanitizerType::Html,
    },
];

static JAVA_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["new Scanner(System.in)"],
        source_type: TaintSourceType::Stdin,
    },
    AstSourcePattern {
        call_names: &["readLine"],
        member_patterns: &["new BufferedReader"],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["request.getParameter", "getQueryString"],
        source_type: TaintSourceType::HttpParam,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["System.getenv"],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["new FileReader", "Files.readAllLines"],
        source_type: TaintSourceType::FileRead,
    },
];

static JAVA_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Runtime.getRuntime().exec", "ProcessBuilder"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &[".execute(", ".executeQuery(", ".executeUpdate("],
        sink_type: TaintSinkType::SqlQuery,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Class.forName"],
        sink_type: TaintSinkType::CodeEval,
    },
];

static JAVA_AST_SANITIZERS: &[AstSanitizerPattern] = &[
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["Integer.parseInt", "Long.parseLong", "Double.parseDouble"],
        sanitizer_type: SanitizerType::Numeric,
    },
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["ESAPI.encoder", "StringEscapeUtils.escapeHtml"],
        sanitizer_type: SanitizerType::Html,
    },
];

static RUST_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["io::stdin", "std::io::stdin"],
        source_type: TaintSourceType::Stdin,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["env::var", "std::env::var"],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["env::args", "std::env::args"],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &[
            "fs::read_to_string",
            "std::fs::read_to_string",
            "File::open",
        ],
        source_type: TaintSourceType::FileRead,
    },
];

static RUST_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Command::new", "std::process::Command"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["unsafe"],
        sink_type: TaintSinkType::CodeEval,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["std::ptr::write", "std::ptr::read"],
        sink_type: TaintSinkType::FileWrite,
    },
];

static RUST_AST_SANITIZERS: &[AstSanitizerPattern] = &[AstSanitizerPattern {
    call_names: &[],
    member_patterns: &[
        ".parse::<i32>",
        ".parse::<i64>",
        ".parse::<u32>",
        ".parse::<u64>",
        ".parse::<f32>",
        ".parse::<f64>",
        ".parse::<usize>",
        ".parse::<isize>",
    ],
    sanitizer_type: SanitizerType::Numeric,
}];

static C_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &["scanf", "fscanf", "sscanf", "fgets", "gets", "getchar"],
        member_patterns: &[],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &["getenv"],
        member_patterns: &[],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &["fread", "fopen"],
        member_patterns: &[],
        source_type: TaintSourceType::FileRead,
    },
    AstSourcePattern {
        call_names: &["recv", "recvfrom"],
        member_patterns: &[],
        source_type: TaintSourceType::UserInput,
    },
];

static C_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &["system", "popen", "execl", "execv", "execvp"],
        member_patterns: &[],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &["sprintf", "vsprintf"],
        member_patterns: &[],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &["strcpy", "strcat", "strncpy"],
        member_patterns: &[],
        sink_type: TaintSinkType::FileWrite,
    },
];

static C_AST_SANITIZERS: &[AstSanitizerPattern] = &[
    AstSanitizerPattern {
        call_names: &["atoi", "atol", "atof", "strtol", "strtoul", "strtod"],
        member_patterns: &[],
        sanitizer_type: SanitizerType::Numeric,
    },
    AstSanitizerPattern {
        call_names: &["snprintf"],
        member_patterns: &[],
        sanitizer_type: SanitizerType::Shell,
    },
];

static CPP_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &["getline"],
        member_patterns: &["std::cin", "std::getline"],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &["getenv"],
        member_patterns: &[],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["std::ifstream", "std::fstream"],
        source_type: TaintSourceType::FileRead,
    },
];

static CPP_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &["system", "popen"],
        member_patterns: &["std::system"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &["sprintf"],
        member_patterns: &[],
        sink_type: TaintSinkType::ShellExec,
    },
];

static CPP_AST_SANITIZERS: &[AstSanitizerPattern] = &[
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &[
            "std::stoi",
            "std::stol",
            "std::stoul",
            "std::stoll",
            "std::stof",
            "std::stod",
        ],
        sanitizer_type: SanitizerType::Numeric,
    },
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &[
            "static_cast<int>",
            "static_cast<long>",
            "static_cast<float>",
            "static_cast<double>",
        ],
        sanitizer_type: SanitizerType::Numeric,
    },
];

static RUBY_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &["gets"],
        member_patterns: &[],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["STDIN.read", "STDIN.gets", "STDIN.readline"],
        source_type: TaintSourceType::Stdin,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["params["],
        source_type: TaintSourceType::HttpParam,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["ENV["],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["File.read", "File.open"],
        source_type: TaintSourceType::FileRead,
    },
];

static RUBY_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &["eval"],
        member_patterns: &[],
        sink_type: TaintSinkType::CodeEval,
    },
    AstSinkPattern {
        call_names: &["system", "exec"],
        member_patterns: &[],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["IO.popen"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &[".send("],
        sink_type: TaintSinkType::CodeEval,
    },
];

static RUBY_AST_SANITIZERS: &[AstSanitizerPattern] = &[
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &[".to_i", ".to_f"],
        sanitizer_type: SanitizerType::Numeric,
    },
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["CGI.escapeHTML", "Rack::Utils.escape_html"],
        sanitizer_type: SanitizerType::Html,
    },
];

static KOTLIN_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &["readLine", "readln"],
        member_patterns: &[],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["System.getenv"],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["BufferedReader"],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["request.getParameter"],
        source_type: TaintSourceType::HttpParam,
    },
];

static KOTLIN_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Runtime.getRuntime().exec", "ProcessBuilder"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &[".execute(", ".executeQuery(", "prepareStatement"],
        sink_type: TaintSinkType::SqlQuery,
    },
];

static KOTLIN_AST_SANITIZERS: &[AstSanitizerPattern] = &[AstSanitizerPattern {
    call_names: &[],
    member_patterns: &[".toInt()", ".toLong()", ".toDouble()", ".toFloat()"],
    sanitizer_type: SanitizerType::Numeric,
}];

static SWIFT_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &["readLine"],
        member_patterns: &[],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["ProcessInfo.processInfo.environment"],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["FileManager.default", "URLSession"],
        source_type: TaintSourceType::FileRead,
    },
];

static SWIFT_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Process()", "NSTask"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &["sqlite3_exec"],
        member_patterns: &[],
        sink_type: TaintSinkType::SqlQuery,
    },
];

static SWIFT_AST_SANITIZERS: &[AstSanitizerPattern] = &[
    AstSanitizerPattern {
        call_names: &["Int", "Double", "Float"],
        member_patterns: &[],
        sanitizer_type: SanitizerType::Numeric,
    },
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["addingPercentEncoding"],
        sanitizer_type: SanitizerType::Html,
    },
];

static CSHARP_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["Console.ReadLine"],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["Request.QueryString", "Request.Form"],
        source_type: TaintSourceType::HttpParam,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["Environment.GetEnvironmentVariable"],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &[
            "File.ReadAllText",
            "File.ReadAllLines",
            "File.OpenRead",
            "StreamReader",
        ],
        source_type: TaintSourceType::FileRead,
    },
];

static CSHARP_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Process.Start"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["SqlCommand", ".ExecuteNonQuery", ".ExecuteReader"],
        sink_type: TaintSinkType::SqlQuery,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Activator.CreateInstance"],
        sink_type: TaintSinkType::CodeEval,
    },
];

static CSHARP_AST_SANITIZERS: &[AstSanitizerPattern] = &[
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["int.Parse", "Convert.ToInt32", "double.Parse"],
        sanitizer_type: SanitizerType::Numeric,
    },
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["HttpUtility.HtmlEncode"],
        sanitizer_type: SanitizerType::Html,
    },
];

static SCALA_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["StdIn.readLine", "scala.io.StdIn"],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["System.getenv"],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["Source.fromFile"],
        source_type: TaintSourceType::FileRead,
    },
];

static SCALA_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Runtime.getRuntime.exec", "sys.process", "Process("],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &[".execute(", ".executeQuery("],
        sink_type: TaintSinkType::SqlQuery,
    },
];

static SCALA_AST_SANITIZERS: &[AstSanitizerPattern] = &[
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &[".toInt", ".toLong", ".toDouble"],
        sanitizer_type: SanitizerType::Numeric,
    },
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["StringEscapeUtils.escapeHtml"],
        sanitizer_type: SanitizerType::Html,
    },
];

static PHP_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["$_GET[", "$_REQUEST[", "$_COOKIE[", "$_SERVER["],
        source_type: TaintSourceType::HttpParam,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["$_POST["],
        source_type: TaintSourceType::HttpBody,
    },
    AstSourcePattern {
        call_names: &["fgets"],
        member_patterns: &[],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &["file_get_contents"],
        member_patterns: &[],
        source_type: TaintSourceType::FileRead,
    },
    AstSourcePattern {
        call_names: &["getenv"],
        member_patterns: &["$_ENV["],
        source_type: TaintSourceType::EnvVar,
    },
];

static PHP_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &["eval"],
        member_patterns: &[],
        sink_type: TaintSinkType::CodeEval,
    },
    AstSinkPattern {
        call_names: &[
            "exec",
            "system",
            "passthru",
            "shell_exec",
            "popen",
            "proc_open",
        ],
        member_patterns: &[],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &["mysqli_query"],
        member_patterns: &["->query("],
        sink_type: TaintSinkType::SqlQuery,
    },
];

static PHP_AST_SANITIZERS: &[AstSanitizerPattern] = &[
    AstSanitizerPattern {
        call_names: &["intval", "floatval"],
        member_patterns: &["(int)", "(float)"],
        sanitizer_type: SanitizerType::Numeric,
    },
    AstSanitizerPattern {
        call_names: &["htmlspecialchars", "htmlentities"],
        member_patterns: &[],
        sanitizer_type: SanitizerType::Html,
    },
    AstSanitizerPattern {
        call_names: &["mysqli_real_escape_string"],
        member_patterns: &[],
        sanitizer_type: SanitizerType::Shell,
    },
];

static LUA_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["io.read"],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["os.getenv"],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["io.open"],
        source_type: TaintSourceType::FileRead,
    },
];

static LUA_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["os.execute"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["io.popen"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &["loadstring", "load", "dofile", "loadfile"],
        member_patterns: &[],
        sink_type: TaintSinkType::CodeEval,
    },
];

static LUA_AST_SANITIZERS: &[AstSanitizerPattern] = &[AstSanitizerPattern {
    call_names: &["tonumber"],
    member_patterns: &[],
    sanitizer_type: SanitizerType::Numeric,
}];

static ELIXIR_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["IO.gets"],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["System.get_env"],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["File.read", "File.read!"],
        source_type: TaintSourceType::FileRead,
    },
];

static ELIXIR_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["System.cmd"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Code.eval_string"],
        sink_type: TaintSinkType::CodeEval,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Ecto.Adapters.SQL.query"],
        sink_type: TaintSinkType::SqlQuery,
    },
];

static ELIXIR_AST_SANITIZERS: &[AstSanitizerPattern] = &[
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["String.to_integer", "String.to_float"],
        sanitizer_type: SanitizerType::Numeric,
    },
    AstSanitizerPattern {
        call_names: &[],
        member_patterns: &["Phoenix.HTML.html_escape"],
        sanitizer_type: SanitizerType::Html,
    },
];

static OCAML_AST_SOURCES: &[AstSourcePattern] = &[
    AstSourcePattern {
        call_names: &["read_line"],
        member_patterns: &[],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &["input_line"],
        member_patterns: &[],
        source_type: TaintSourceType::UserInput,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["Sys.getenv"],
        source_type: TaintSourceType::EnvVar,
    },
    AstSourcePattern {
        call_names: &[],
        member_patterns: &["In_channel.read_all", "In_channel.input_all"],
        source_type: TaintSourceType::FileRead,
    },
];

static OCAML_AST_SINKS: &[AstSinkPattern] = &[
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Sys.command"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Unix.execvp"],
        sink_type: TaintSinkType::ShellExec,
    },
    AstSinkPattern {
        call_names: &[],
        member_patterns: &["Sqlite3.exec"],
        sink_type: TaintSinkType::SqlQuery,
    },
];

static OCAML_AST_SANITIZERS: &[AstSanitizerPattern] = &[AstSanitizerPattern {
    call_names: &["int_of_string", "float_of_string"],
    member_patterns: &[],
    sanitizer_type: SanitizerType::Numeric,
}];

/// Get AST-based taint patterns for a given language.
fn get_ast_patterns(language: Language) -> AstLanguagePatterns {
    match language {
        Language::Python => AstLanguagePatterns {
            sources: PYTHON_AST_SOURCES,
            sinks: PYTHON_AST_SINKS,
            sanitizers: PYTHON_AST_SANITIZERS,
        },
        Language::TypeScript | Language::JavaScript => AstLanguagePatterns {
            sources: TYPESCRIPT_AST_SOURCES,
            sinks: TYPESCRIPT_AST_SINKS,
            sanitizers: TYPESCRIPT_AST_SANITIZERS,
        },
        Language::Go => AstLanguagePatterns {
            sources: GO_AST_SOURCES,
            sinks: GO_AST_SINKS,
            sanitizers: GO_AST_SANITIZERS,
        },
        Language::Java => AstLanguagePatterns {
            sources: JAVA_AST_SOURCES,
            sinks: JAVA_AST_SINKS,
            sanitizers: JAVA_AST_SANITIZERS,
        },
        Language::Rust => AstLanguagePatterns {
            sources: RUST_AST_SOURCES,
            sinks: RUST_AST_SINKS,
            sanitizers: RUST_AST_SANITIZERS,
        },
        Language::C => AstLanguagePatterns {
            sources: C_AST_SOURCES,
            sinks: C_AST_SINKS,
            sanitizers: C_AST_SANITIZERS,
        },
        Language::Cpp => AstLanguagePatterns {
            sources: CPP_AST_SOURCES,
            sinks: CPP_AST_SINKS,
            sanitizers: CPP_AST_SANITIZERS,
        },
        Language::Ruby => AstLanguagePatterns {
            sources: RUBY_AST_SOURCES,
            sinks: RUBY_AST_SINKS,
            sanitizers: RUBY_AST_SANITIZERS,
        },
        Language::Kotlin => AstLanguagePatterns {
            sources: KOTLIN_AST_SOURCES,
            sinks: KOTLIN_AST_SINKS,
            sanitizers: KOTLIN_AST_SANITIZERS,
        },
        Language::Swift => AstLanguagePatterns {
            sources: SWIFT_AST_SOURCES,
            sinks: SWIFT_AST_SINKS,
            sanitizers: SWIFT_AST_SANITIZERS,
        },
        Language::CSharp => AstLanguagePatterns {
            sources: CSHARP_AST_SOURCES,
            sinks: CSHARP_AST_SINKS,
            sanitizers: CSHARP_AST_SANITIZERS,
        },
        Language::Scala => AstLanguagePatterns {
            sources: SCALA_AST_SOURCES,
            sinks: SCALA_AST_SINKS,
            sanitizers: SCALA_AST_SANITIZERS,
        },
        Language::Php => AstLanguagePatterns {
            sources: PHP_AST_SOURCES,
            sinks: PHP_AST_SINKS,
            sanitizers: PHP_AST_SANITIZERS,
        },
        Language::Lua | Language::Luau => AstLanguagePatterns {
            sources: LUA_AST_SOURCES,
            sinks: LUA_AST_SINKS,
            sanitizers: LUA_AST_SANITIZERS,
        },
        Language::Elixir => AstLanguagePatterns {
            sources: ELIXIR_AST_SOURCES,
            sinks: ELIXIR_AST_SINKS,
            sanitizers: ELIXIR_AST_SANITIZERS,
        },
        Language::Ocaml => AstLanguagePatterns {
            sources: OCAML_AST_SOURCES,
            sinks: OCAML_AST_SINKS,
            sanitizers: OCAML_AST_SANITIZERS,
        },
    }
}

// ---------------------------------------------------------------------------
// AST-Based Detection Functions
// ---------------------------------------------------------------------------

/// Detect taint sources using AST nodes from a parsed tree.
///
/// Walks the tree looking for call nodes that match known source patterns.
/// Unlike regex-based detection, this correctly skips matches inside
/// comments and string literals.
///
/// # Arguments
/// * `root` - Root node of the function/file to analyze
/// * `source` - Source code bytes
/// * `language` - Programming language
/// * `line_filter` - If Some, only detect sources on this specific line
///
/// # Returns
/// Vector of detected taint sources
pub fn detect_sources_ast(
    root: &tree_sitter::Node,
    source: &[u8],
    language: Language,
    line_filter: Option<u32>,
) -> Vec<TaintSource> {
    let patterns = get_ast_patterns(language);
    let mut sources = Vec::new();
    let descendants = walk_descendants(*root);

    for descendant in &descendants {
        // Skip comments and strings
        if is_in_comment(descendant, language) || is_in_string(descendant, language) {
            continue;
        }

        let line = descendant.start_position().row as u32 + 1;
        if let Some(filter) = line_filter {
            if line != filter {
                continue;
            }
        }

        let text = node_text(descendant, source);

        for pattern in patterns.sources {
            let matched = pattern.call_names.iter().any(|name| {
                // Check if this is a call node with matching name
                let call_kinds = call_node_kinds(language);
                if call_kinds.contains(&descendant.kind()) {
                    if let Some(call_name) = extract_call_name(descendant, source, language) {
                        return call_name == *name || call_name.ends_with(&format!(".{}", name));
                    }
                }
                false
            }) || pattern.member_patterns.iter().any(|mp| text.contains(mp));

            if matched {
                // Try to get variable from parent assignment
                let var = find_parent_assignment_var(descendant, source, language).or_else(|| {
                    extract_assigned_var(
                        std::str::from_utf8(source)
                            .unwrap_or("")
                            .lines()
                            .nth((line - 1) as usize)
                            .unwrap_or(""),
                    )
                });

                if let Some(var) = var {
                    sources.push(TaintSource {
                        var,
                        line,
                        source_type: pattern.source_type,
                        statement: Some(
                            std::str::from_utf8(source)
                                .unwrap_or("")
                                .lines()
                                .nth((line - 1) as usize)
                                .unwrap_or("")
                                .to_string(),
                        ),
                    });
                    break; // Only one source per node
                }
            }
        }
    }

    sources
}

/// Detect taint sinks using AST nodes from a parsed tree.
///
/// Similar to `detect_sources_ast` but for dangerous operations (sinks).
pub fn detect_sinks_ast(
    root: &tree_sitter::Node,
    source: &[u8],
    language: Language,
    line_filter: Option<u32>,
) -> Vec<TaintSink> {
    let patterns = get_ast_patterns(language);
    let mut sinks = Vec::new();
    let descendants = walk_descendants(*root);

    for descendant in &descendants {
        if is_in_comment(descendant, language) || is_in_string(descendant, language) {
            continue;
        }

        let line = descendant.start_position().row as u32 + 1;
        if let Some(filter) = line_filter {
            if line != filter {
                continue;
            }
        }

        let text = node_text(descendant, source);

        for pattern in patterns.sinks {
            let matched = pattern.call_names.iter().any(|name| {
                let call_kinds = call_node_kinds(language);
                if call_kinds.contains(&descendant.kind()) {
                    if let Some(call_name) = extract_call_name(descendant, source, language) {
                        return call_name == *name || call_name.ends_with(&format!(".{}", name));
                    }
                }
                false
            }) || pattern.member_patterns.iter().any(|mp| text.contains(mp));

            if matched {
                let stmt_text = std::str::from_utf8(source)
                    .unwrap_or("")
                    .lines()
                    .nth((line - 1) as usize)
                    .unwrap_or("");

                // Extract variable argument
                let regex_patterns = get_patterns(language);
                let var = regex_patterns
                    .sinks
                    .iter()
                    .find(|(p, _)| p.is_match(stmt_text))
                    .and_then(|(p, _)| extract_call_arg(stmt_text, p))
                    .or_else(|| {
                        regex_patterns
                            .sinks
                            .iter()
                            .find(|(p, _)| p.is_match(stmt_text))
                            .and_then(|(p, _)| extract_sink_var_from_statement(stmt_text, p))
                    });

                if let Some(var) = var {
                    sinks.push(TaintSink {
                        var,
                        line,
                        sink_type: pattern.sink_type,
                        tainted: false,
                        statement: Some(stmt_text.to_string()),
                    });
                    break;
                }
            }
        }
    }

    sinks
}

/// Detect sanitizers using AST nodes.
///
/// Returns the sanitizer type if found, checking that the match
/// is in actual code (not in a comment or string).
pub fn detect_sanitizer_ast(
    root: &tree_sitter::Node,
    source: &[u8],
    language: Language,
    line: u32,
) -> Option<SanitizerType> {
    let patterns = get_ast_patterns(language);
    let descendants = walk_descendants(*root);

    for descendant in &descendants {
        if is_in_comment(descendant, language) || is_in_string(descendant, language) {
            continue;
        }

        let node_line = descendant.start_position().row as u32 + 1;
        if node_line != line {
            continue;
        }

        let text = node_text(descendant, source);

        for pattern in patterns.sanitizers {
            let matched = pattern.call_names.iter().any(|name| {
                let call_kinds = call_node_kinds(language);
                if call_kinds.contains(&descendant.kind()) {
                    if let Some(call_name) = extract_call_name(descendant, source, language) {
                        return call_name == *name;
                    }
                }
                false
            }) || pattern.member_patterns.iter().any(|mp| text.contains(mp));

            if matched {
                return Some(pattern.sanitizer_type);
            }
        }
    }

    None
}

/// Compute taint analysis with optional AST tree for improved detection.
///
/// When a parsed tree is provided, uses AST-based detection to filter out
/// false positives from comments and string literals. Falls back to regex
/// when AST detection yields no results.
///
/// This is the preferred entry point for CLI commands that have access to
/// the full parsed tree.
pub fn compute_taint_with_tree(
    cfg: &CfgInfo,
    refs: &[VarRef],
    statements: &HashMap<u32, String>,
    tree: Option<&tree_sitter::Tree>,
    source: Option<&[u8]>,
    language: Language,
) -> Result<TaintInfo, TldrError> {
    // If we have tree + source, use AST-enhanced detection within compute_taint
    // For now, delegate to the existing compute_taint which uses regex patterns.
    // The AST detection functions are available for direct use, and we integrate
    // them here as an enhancement layer.

    // Validate CFG
    validate_cfg(cfg)?;

    let mut result = TaintInfo::new(&cfg.function);

    // Build helper maps
    let predecessors = build_predecessors(cfg);
    let successors = build_successors(cfg);
    let line_to_block = build_line_to_block(cfg);
    let refs_by_block = build_refs_by_block(refs, &line_to_block);

    // Detect sources and sinks
    if let (Some(tree), Some(src)) = (tree, source) {
        // AST-based detection: walk the tree ONCE (no line filter) to avoid
        // O(lines * nodes) quadratic slowdown that caused infinite-loop-like hangs
        // on large files.
        let root = tree.root_node();

        let all_ast_sources = detect_sources_ast(&root, src, language, None);
        let all_ast_sinks = detect_sinks_ast(&root, src, language, None);

        // Index AST results by line for fast lookup
        let mut ast_sources_by_line: HashMap<u32, Vec<TaintSource>> = HashMap::new();
        for s in all_ast_sources {
            ast_sources_by_line.entry(s.line).or_default().push(s);
        }
        let mut ast_sinks_by_line: HashMap<u32, Vec<TaintSink>> = HashMap::new();
        for s in all_ast_sinks {
            ast_sinks_by_line.entry(s.line).or_default().push(s);
        }

        for (&line, stmt) in statements {
            // Sources: prefer AST results, fall back to regex
            if let Some(sources) = ast_sources_by_line.remove(&line) {
                result.sources.extend(sources);
            } else {
                result.sources.extend(detect_sources(stmt, line, language));
            }

            // Sinks: merge AST and regex results to avoid missing detections
            // when AST finds something on a line but misses certain sink patterns.
            // Dedup below handles any duplicates from the merge.
            if let Some(sinks) = ast_sinks_by_line.remove(&line) {
                result.sinks.extend(sinks);
            }
            result.sinks.extend(detect_sinks(stmt, line, language));
        }
    } else {
        // No tree available - use regex only (backward compatible)
        for (&line, stmt) in statements {
            result.sources.extend(detect_sources(stmt, line, language));
            result.sinks.extend(detect_sinks(stmt, line, language));
        }
    }

    // Deduplicate sources by (line, source_type, var)
    result.sources.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then_with(|| format!("{:?}", a.source_type).cmp(&format!("{:?}", b.source_type)))
            .then_with(|| a.var.cmp(&b.var))
    });
    result.sources.dedup_by(|a, b| {
        a.line == b.line
            && a.var == b.var
            && std::mem::discriminant(&a.source_type) == std::mem::discriminant(&b.source_type)
    });

    // Deduplicate sinks by (line, sink_type, var)
    result.sinks.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then_with(|| format!("{:?}", a.sink_type).cmp(&format!("{:?}", b.sink_type)))
            .then_with(|| a.var.cmp(&b.var))
    });
    result.sinks.dedup_by(|a, b| {
        a.line == b.line
            && a.var == b.var
            && std::mem::discriminant(&a.sink_type) == std::mem::discriminant(&b.sink_type)
    });

    // The rest of the algorithm is the same as compute_taint

    // Initialize taint sets per block
    let block_ids: Vec<usize> = cfg.blocks.iter().map(|b| b.id).collect();
    let mut tainted: HashMap<usize, HashSet<String>> = HashMap::new();
    for &bid in &block_ids {
        tainted.insert(bid, HashSet::new());
    }

    for source in &result.sources {
        if let Some(&block_id) = line_to_block.get(&source.line) {
            tainted
                .entry(block_id)
                .or_default()
                .insert(source.var.clone());
        }
    }

    // Worklist iteration
    // Cap iterations to prevent infinite loops on large real-world files
    let unique_vars: HashSet<&str> = refs.iter().map(|r| r.name.as_str()).collect();
    let computed_max = block_ids.len() * unique_vars.len().max(1) + 10;
    let max_iterations = computed_max.min(MAX_TAINT_ITERATIONS);
    let mut worklist: VecDeque<usize> = block_ids.iter().cloned().collect();
    let mut iterations = 0;
    let mut iteration_limit_reached = false;

    let mut source_vars_by_block: HashMap<usize, HashSet<String>> = HashMap::new();
    for source in &result.sources {
        if let Some(&block_id) = line_to_block.get(&source.line) {
            source_vars_by_block
                .entry(block_id)
                .or_default()
                .insert(source.var.clone());
        }
    }

    while let Some(block_id) = worklist.pop_front() {
        if iterations >= max_iterations {
            iteration_limit_reached = true;
            break;
        }
        iterations += 1;

        let mut taint_in: HashSet<String> = predecessors
            .get(&block_id)
            .map(|preds| {
                preds
                    .iter()
                    .flat_map(|p| tainted.get(p).cloned().unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default();

        if let Some(source_vars) = source_vars_by_block.get(&block_id) {
            taint_in.extend(source_vars.clone());
        }

        let taint_out = process_block(
            block_id,
            taint_in,
            &refs_by_block,
            statements,
            &line_to_block,
            &mut result.sanitized_vars,
            language,
        );

        let old_taint = tainted.get(&block_id).cloned().unwrap_or_default();
        if taint_out != old_taint {
            tainted.insert(block_id, taint_out);
            if let Some(succs) = successors.get(&block_id) {
                for &s in succs {
                    if !worklist.contains(&s) {
                        worklist.push_back(s);
                    }
                }
            }
        }
    }

    if iteration_limit_reached {
        result.convergence = Some("iteration_limit_reached".to_string());
    }

    result.tainted_vars = tainted.clone();

    // Phase 5: Detect vulnerabilities
    for sink in &mut result.sinks {
        if let Some(&sink_block) = line_to_block.get(&sink.line) {
            if let Some(tainted_at_block) = tainted.get(&sink_block) {
                // Direct match: sink variable itself is tainted
                if tainted_at_block.contains(&sink.var) {
                    sink.tainted = true;
                } else if !tainted_at_block.is_empty() {
                    // Indirect match: check if any tainted variable appears
                    // in the block's statements. Handles multi-line calls where
                    // the tainted argument is on a different line than the sink
                    // function name (e.g., conn.execute(\n "..." + username))
                    if let Some(block) = cfg.blocks.iter().find(|b| b.id == sink_block) {
                        let block_text: String = (block.lines.0..=block.lines.1)
                            .filter_map(|l| statements.get(&l))
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        for tvar in tainted_at_block {
                            if identifier_in_text(&block_text, tvar) {
                                sink.tainted = true;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    let sources_clone = result.sources.clone();
    let sinks_snapshot: Vec<(String, u32, TaintSinkType, bool, Option<String>)> = result
        .sinks
        .iter()
        .map(|s| {
            (
                s.var.clone(),
                s.line,
                s.sink_type,
                s.tainted,
                s.statement.clone(),
            )
        })
        .collect();

    for (sink_var, sink_line, sink_type, sink_tainted, sink_statement) in sinks_snapshot {
        if !sink_tainted {
            continue;
        }

        if let Some(&sink_block) = line_to_block.get(&sink_line) {
            for source in &sources_clone {
                if let Some(&source_block) = line_to_block.get(&source.line) {
                    if flows_to(&source.var, &sink_var, &tainted, &predecessors, sink_block) {
                        let is_sanitized = result.sanitized_vars.contains(&sink_var);
                        if !is_sanitized {
                            let path = compute_flow_path(source_block, sink_block, &successors);
                            let flow = TaintFlow {
                                source: source.clone(),
                                sink: TaintSink {
                                    var: sink_var.clone(),
                                    line: sink_line,
                                    sink_type,
                                    tainted: true,
                                    statement: sink_statement.clone(),
                                },
                                path,
                            };
                            result.flows.push(flow);
                        }
                    }
                }
            }
        }
    }

    Ok(result)
}

// =============================================================================
// Vulnerability Detection Helpers - Phase 5
// =============================================================================

/// Check if source variable flows to target variable via taint propagation.
///
/// This is a conservative check that assumes any source could cause taint
/// if the target variable is tainted at the target block. A more precise
/// implementation would track per-variable taint provenance.
///
/// # Arguments
///
/// * `_source_var` - The source variable (unused in conservative check)
/// * `target_var` - The variable to check at the sink
/// * `tainted_vars` - Taint state at each block
/// * `_predecessors` - Block predecessor map (unused in conservative check)
/// * `target_block` - The block containing the sink
///
/// # Returns
///
/// `true` if the target variable is tainted at the target block.
fn flows_to(
    _source_var: &str,
    target_var: &str,
    tainted_vars: &HashMap<usize, HashSet<String>>,
    _predecessors: &HashMap<usize, Vec<usize>>,
    target_block: usize,
) -> bool {
    // Conservative approximation: if target_var is tainted at target_block,
    // assume any source could cause it. More precise tracking would require
    // per-variable taint provenance.
    tainted_vars
        .get(&target_block)
        .map(|t| t.contains(target_var))
        .unwrap_or(false)
}

/// Compute block IDs along the flow path from source to sink.
///
/// Uses BFS to find the shortest path through the CFG from the block
/// containing the source to the block containing the sink.
///
/// # Arguments
///
/// * `source_block` - Block ID containing the taint source
/// * `sink_block` - Block ID containing the taint sink
/// * `successors` - Block successor map
///
/// # Returns
///
/// Vector of block IDs from source to sink (inclusive).
fn compute_flow_path(
    source_block: usize,
    sink_block: usize,
    successors: &HashMap<usize, Vec<usize>>,
) -> Vec<usize> {
    if source_block == sink_block {
        return vec![source_block];
    }

    // BFS to find shortest path
    let mut visited: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<Vec<usize>> = VecDeque::new();

    queue.push_back(vec![source_block]);
    visited.insert(source_block);

    while let Some(path) = queue.pop_front() {
        let current = *path.last().unwrap();

        if let Some(succs) = successors.get(&current) {
            for &next in succs {
                if next == sink_block {
                    let mut result = path.clone();
                    result.push(next);
                    return result;
                }

                if !visited.contains(&next) {
                    visited.insert(next);
                    let mut new_path = path.clone();
                    new_path.push(next);
                    queue.push_back(new_path);
                }
            }
        }
    }

    // No path found - return just source and sink
    vec![source_block, sink_block]
}

// =============================================================================
// Worklist Algorithm - Phase 4
// =============================================================================

/// Compute taint analysis for a function using worklist-based forward dataflow.
///
/// # Algorithm
///
/// Forward worklist-based dataflow analysis:
/// 1. Initialize: entry block tainted_vars = sources
/// 2. Worklist iteration until fixed point:
///    - taint_in[B] = union(taint_out[P] for P in predecessors[B])
///    - Process block: propagate taint through assignments
///    - taint_out[B] = process_block(taint_in[B])
///    - If changed, add successors to worklist
///
/// # Arguments
///
/// * `cfg` - Control flow graph for the function
/// * `refs` - Variable references (definitions and uses)
/// * `statements` - Map of line number to statement text (for pattern matching)
/// * `language` - The programming language (determines which taint patterns to use)
///
/// # Returns
///
/// `TaintInfo` containing all taint analysis results.
///
/// # Errors
///
/// Returns `TldrError::InvalidArgs` if the CFG is invalid.
pub fn compute_taint(
    cfg: &CfgInfo,
    refs: &[VarRef],
    statements: &HashMap<u32, String>,
    language: Language,
) -> Result<TaintInfo, TldrError> {
    // Validate CFG
    validate_cfg(cfg)?;

    let mut result = TaintInfo::new(&cfg.function);

    // Build helper maps
    let predecessors = build_predecessors(cfg);
    let successors = build_successors(cfg);
    let line_to_block = build_line_to_block(cfg);
    let refs_by_block = build_refs_by_block(refs, &line_to_block);

    // Detect sources and sinks from statements
    for (&line, stmt) in statements {
        for source in detect_sources(stmt, line, language) {
            result.sources.push(source);
        }
        for sink in detect_sinks(stmt, line, language) {
            result.sinks.push(sink);
        }
    }

    // Initialize taint sets per block
    let block_ids: Vec<usize> = cfg.blocks.iter().map(|b| b.id).collect();
    let mut tainted: HashMap<usize, HashSet<String>> = HashMap::new();
    for &bid in &block_ids {
        tainted.insert(bid, HashSet::new());
    }

    // Initialize all blocks with their respective source variables
    for source in &result.sources {
        if let Some(&block_id) = line_to_block.get(&source.line) {
            tainted
                .entry(block_id)
                .or_default()
                .insert(source.var.clone());
        }
    }

    // Worklist iteration
    // Max iterations bounded by O(blocks * vars) with hard cap to guarantee termination
    let unique_vars: HashSet<&str> = refs.iter().map(|r| r.name.as_str()).collect();
    let computed_max = block_ids.len() * unique_vars.len().max(1) + 10;
    let max_iterations = computed_max.min(MAX_TAINT_ITERATIONS);
    let mut worklist: VecDeque<usize> = block_ids.iter().cloned().collect();
    let mut iterations = 0;
    let mut iteration_limit_reached = false;

    // Build a map of source variables by block for initialization
    let mut source_vars_by_block: HashMap<usize, HashSet<String>> = HashMap::new();
    for source in &result.sources {
        if let Some(&block_id) = line_to_block.get(&source.line) {
            source_vars_by_block
                .entry(block_id)
                .or_default()
                .insert(source.var.clone());
        }
    }

    while let Some(block_id) = worklist.pop_front() {
        if iterations >= max_iterations {
            iteration_limit_reached = true;
            break; // Safety bound to prevent infinite loops
        }
        iterations += 1;

        // Compute taint_in = union of predecessors' taint_out
        let mut taint_in: HashSet<String> = predecessors
            .get(&block_id)
            .map(|preds| {
                preds
                    .iter()
                    .flat_map(|p| tainted.get(p).cloned().unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default();

        // Add source variables that originate in this block
        if let Some(source_vars) = source_vars_by_block.get(&block_id) {
            taint_in.extend(source_vars.clone());
        }

        // Process block: propagate taint through assignments
        let taint_out = process_block(
            block_id,
            taint_in,
            &refs_by_block,
            statements,
            &line_to_block,
            &mut result.sanitized_vars,
            language,
        );

        // If changed, add successors to worklist
        let old_taint = tainted.get(&block_id).cloned().unwrap_or_default();
        if taint_out != old_taint {
            tainted.insert(block_id, taint_out);
            if let Some(succs) = successors.get(&block_id) {
                for &s in succs {
                    if !worklist.contains(&s) {
                        worklist.push_back(s);
                    }
                }
            }
        }
    }

    if iteration_limit_reached {
        result.convergence = Some("iteration_limit_reached".to_string());
    }

    result.tainted_vars = tainted.clone();

    // =========================================================================
    // Phase 5: Detect vulnerabilities (source -> sink flows)
    // =========================================================================
    // For each sink, check if its variable is tainted at that block.
    // If tainted, find the source(s) that caused it and record the flow.

    for sink in &mut result.sinks {
        // Get block containing this sink
        if let Some(&sink_block) = line_to_block.get(&sink.line) {
            // Check if sink variable is tainted at this point
            if let Some(tainted_at_block) = tainted.get(&sink_block) {
                if tainted_at_block.contains(&sink.var) {
                    sink.tainted = true;
                }
            }
        }
    }

    // Now create flows for tainted sinks
    // We need to iterate over sinks again with immutable access to sources
    let sources_clone = result.sources.clone();
    let sinks_snapshot: Vec<(String, u32, TaintSinkType, bool, Option<String>)> = result
        .sinks
        .iter()
        .map(|s| {
            (
                s.var.clone(),
                s.line,
                s.sink_type,
                s.tainted,
                s.statement.clone(),
            )
        })
        .collect();

    for (sink_var, sink_line, sink_type, sink_tainted, sink_statement) in sinks_snapshot {
        if !sink_tainted {
            continue;
        }

        // Get block containing this sink
        if let Some(&sink_block) = line_to_block.get(&sink_line) {
            // Find which source(s) caused this taint
            for source in &sources_clone {
                // Get block containing this source
                if let Some(&source_block) = line_to_block.get(&source.line) {
                    // Check if source variable flows to sink variable
                    if flows_to(&source.var, &sink_var, &tainted, &predecessors, sink_block) {
                        // Check if the sink variable was sanitized
                        let is_sanitized = result.sanitized_vars.contains(&sink_var);

                        // Only record if NOT sanitized
                        if !is_sanitized {
                            let path = compute_flow_path(source_block, sink_block, &successors);

                            let flow = TaintFlow {
                                source: source.clone(),
                                sink: TaintSink {
                                    var: sink_var.clone(),
                                    line: sink_line,
                                    sink_type,
                                    tainted: true,
                                    statement: sink_statement.clone(),
                                },
                                path,
                            };

                            result.flows.push(flow);
                        }
                    }
                }
            }
        }
    }

    Ok(result)
}

/// Process a single block for taint propagation.
///
/// Propagates taint through assignments in the block:
/// - If RHS uses a tainted variable, LHS becomes tainted
/// - If a sanitizer is applied, the result is NOT tainted
/// - Definitions without taint remove taint from the variable
///
/// # Arguments
///
/// * `block_id` - The block being processed
/// * `current_taint` - Set of tainted variables at block entry
/// * `refs_by_block` - VarRefs grouped by block
/// * `statements` - Statement text by line number
/// * `line_to_block` - Mapping from line to block ID
/// * `sanitized_vars` - Set of variables that have been sanitized (mutated)
/// * `language` - The programming language (determines which patterns to use)
///
/// # Returns
///
/// Set of tainted variables at block exit.
fn process_block(
    block_id: usize,
    mut current_taint: HashSet<String>,
    refs_by_block: &HashMap<usize, Vec<&VarRef>>,
    statements: &HashMap<u32, String>,
    _line_to_block: &HashMap<u32, usize>,
    sanitized_vars: &mut HashSet<String>,
    language: Language,
) -> HashSet<String> {
    let empty_refs = vec![];
    let block_refs = refs_by_block.get(&block_id).unwrap_or(&empty_refs);

    for var_ref in block_refs {
        let stmt = statements
            .get(&var_ref.line)
            .map(|s| s.as_str())
            .unwrap_or("");

        match var_ref.ref_type {
            RefType::Definition => {
                // Check if RHS uses a tainted variable
                let rhs_tainted = current_taint.iter().any(|tv| stmt.contains(tv.as_str()));

                // Check if sanitized
                if detect_sanitizer(stmt, language).is_some() {
                    sanitized_vars.insert(var_ref.name.clone());
                    current_taint.remove(&var_ref.name);
                } else if rhs_tainted {
                    current_taint.insert(var_ref.name.clone());
                } else {
                    // Definition without taint removes taint
                    current_taint.remove(&var_ref.name);
                }
            }
            RefType::Use => {
                // Uses don't change taint state directly
            }
            RefType::Update => {
                // Update is use-then-def (e.g., x += y)
                // If RHS is tainted, result is tainted
                let rhs_tainted = current_taint.iter().any(|tv| stmt.contains(tv.as_str()));
                if rhs_tainted {
                    current_taint.insert(var_ref.name.clone());
                }
            }
        }
    }

    current_taint
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_taint_source_type_serde() {
        let source = TaintSourceType::UserInput;
        let json = serde_json::to_string(&source).unwrap();
        assert_eq!(json, "\"user_input\"");

        let parsed: TaintSourceType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, source);
    }

    #[test]
    fn test_taint_sink_type_serde() {
        let sink = TaintSinkType::SqlQuery;
        let json = serde_json::to_string(&sink).unwrap();
        assert_eq!(json, "\"sql_query\"");

        let parsed: TaintSinkType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, sink);
    }

    #[test]
    fn test_sanitizer_type_serde() {
        let sanitizer = SanitizerType::Numeric;
        let json = serde_json::to_string(&sanitizer).unwrap();
        assert_eq!(json, "\"numeric\"");

        let parsed: SanitizerType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, sanitizer);
    }

    #[test]
    fn test_taint_info_new() {
        let info = TaintInfo::new("my_function");
        assert_eq!(info.function_name, "my_function");
        assert!(info.tainted_vars.is_empty());
        assert!(info.sources.is_empty());
        assert!(info.sinks.is_empty());
        assert!(info.flows.is_empty());
        assert!(info.sanitized_vars.is_empty());
    }

    #[test]
    fn test_taint_info_default() {
        let info = TaintInfo::default();
        assert!(info.function_name.is_empty());
        assert!(info.tainted_vars.is_empty());
    }

    #[test]
    fn test_taint_info_is_tainted() {
        let mut info = TaintInfo::new("test");
        let mut block_taint = HashSet::new();
        block_taint.insert("user_input".to_string());
        info.tainted_vars.insert(0, block_taint);

        assert!(info.is_tainted(0, "user_input"));
        assert!(!info.is_tainted(0, "other_var"));
        assert!(!info.is_tainted(1, "user_input")); // block 1 doesn't exist
    }

    #[test]
    fn test_taint_info_get_vulnerabilities() {
        let mut info = TaintInfo::new("test");

        // Add a tainted sink (vulnerability)
        info.sinks.push(TaintSink {
            var: "query".to_string(),
            line: 5,
            sink_type: TaintSinkType::SqlQuery,
            tainted: true,
            statement: Some("cursor.execute(query)".to_string()),
        });

        // Add a non-tainted sink (safe)
        info.sinks.push(TaintSink {
            var: "safe_query".to_string(),
            line: 10,
            sink_type: TaintSinkType::SqlQuery,
            tainted: false,
            statement: Some("cursor.execute(safe_query)".to_string()),
        });

        let vulns = info.get_vulnerabilities();
        assert_eq!(vulns.len(), 1);
        assert_eq!(vulns[0].var, "query");
    }

    /// Test that compute_taint terminates on a large CFG with many variables
    /// and back-edges that could cause oscillation in the worklist algorithm.
    /// This test would hang forever without the MAX_TAINT_ITERATIONS cap.
    #[test]
    fn test_taint_terminates_on_large_cfg_with_backedges() {
        use crate::types::{BlockType, CfgBlock, CfgEdge, CfgInfo, EdgeType, RefType, VarRef};

        // Create a CFG with 50 blocks in a chain, plus back-edges
        let num_blocks = 50;
        let mut blocks = Vec::new();
        let mut edges = Vec::new();

        for i in 0..num_blocks {
            let start_line = (i * 10 + 1) as u32;
            let end_line = (i * 10 + 10) as u32;
            blocks.push(CfgBlock {
                id: i,
                block_type: BlockType::Body,
                lines: (start_line, end_line),
                calls: Vec::new(),
            });
        }

        // Linear chain edges
        for i in 0..num_blocks - 1 {
            edges.push(CfgEdge {
                from: i,
                to: i + 1,
                edge_type: EdgeType::Unconditional,
                condition: None,
            });
        }

        // Add back-edges to create loops that could cause oscillation
        for i in (5..num_blocks).step_by(5) {
            edges.push(CfgEdge {
                from: i,
                to: i - 3,
                edge_type: EdgeType::BackEdge,
                condition: None,
            });
        }

        let cfg = CfgInfo {
            function: "large_func".to_string(),
            blocks,
            edges,
            entry_block: 0,
            exit_blocks: vec![num_blocks - 1],
            cyclomatic_complexity: 10,
            nested_functions: HashMap::new(),
        };

        // Create many variable refs across blocks
        let mut refs = Vec::new();
        let mut statements = HashMap::new();

        for i in 0..num_blocks {
            let line = (i * 10 + 1) as u32;
            let var_name = format!("var_{}", i);
            refs.push(VarRef {
                name: var_name.clone(),
                ref_type: RefType::Definition,
                line,
                column: 0,
                context: None,
                group_id: None,
            });
            // Create statements that reference previous variables to create taint chains
            if i > 0 {
                statements.insert(line, format!("var_{} = var_{}", i, i - 1));
            } else {
                statements.insert(line, "var_0 = input()".to_string());
            }
        }

        // This MUST terminate within a reasonable time (< 1 second)
        let start = std::time::Instant::now();
        let result = compute_taint(&cfg, &refs, &statements, Language::Python);
        let elapsed = start.elapsed();

        assert!(result.is_ok(), "compute_taint should succeed");
        assert!(
            elapsed.as_secs() < 5,
            "compute_taint took too long: {:?} (possible infinite loop)",
            elapsed
        );

        // Should have found the input() source
        let info = result.unwrap();
        assert!(!info.sources.is_empty(), "Should detect input() source");
    }

    /// Test that the hard iteration cap MAX_TAINT_ITERATIONS is respected
    /// even when the computed max_iterations would be very large.
    #[test]
    fn test_taint_iteration_cap_prevents_runaway() {
        use crate::types::{BlockType, CfgBlock, CfgEdge, CfgInfo, EdgeType, RefType, VarRef};

        // Create a small CFG but with MANY variable references to inflate max_iterations
        let blocks = vec![
            CfgBlock {
                id: 0,
                block_type: BlockType::Body,
                lines: (1, 100),
                calls: Vec::new(),
            },
            CfgBlock {
                id: 1,
                block_type: BlockType::Body,
                lines: (101, 200),
                calls: Vec::new(),
            },
        ];
        let edges = vec![
            CfgEdge {
                from: 0,
                to: 1,
                edge_type: EdgeType::Unconditional,
                condition: None,
            },
            CfgEdge {
                from: 1,
                to: 0,
                edge_type: EdgeType::BackEdge,
                condition: None,
            },
        ];

        let cfg = CfgInfo {
            function: "runaway".to_string(),
            blocks,
            edges,
            entry_block: 0,
            exit_blocks: vec![1],
            cyclomatic_complexity: 2,
            nested_functions: HashMap::new(),
        };

        // Create 500 unique variable refs - this would make max_iterations = 2 * 500 + 10 = 1010
        // which is above our MAX_TAINT_ITERATIONS cap of 1000
        let mut refs = Vec::new();
        let mut statements = HashMap::new();

        for i in 0..500 {
            let line = (i + 1) as u32;
            refs.push(VarRef {
                name: format!("v{}", i),
                ref_type: RefType::Definition,
                line,
                column: 0,
                context: None,
                group_id: None,
            });
            statements.insert(line, format!("v{} = input()", i));
        }

        let start = std::time::Instant::now();
        let result = compute_taint(&cfg, &refs, &statements, Language::Python);
        let elapsed = start.elapsed();

        assert!(result.is_ok());
        assert!(
            elapsed.as_secs() < 5,
            "Should terminate quickly with iteration cap, took {:?}",
            elapsed
        );
    }

    /// Test that compute_taint_with_tree deduplicates sources that are detected
    /// by both AST-based and regex-based detection on the same line.
    #[test]
    fn test_sources_are_deduplicated() {
        use crate::ast::ParserPool;
        use crate::types::{BlockType, CfgBlock, CfgEdge, CfgInfo, EdgeType, RefType, VarRef};

        let python_code = r#"import os

def vulnerable_func(user_input):
    data = input("Enter: ")
    query = "SELECT * FROM users WHERE id = " + data
    os.system(user_input)
    eval(data)
"#;

        let cfg = CfgInfo {
            function: "vulnerable_func".to_string(),
            blocks: vec![
                CfgBlock {
                    id: 0,
                    block_type: BlockType::Entry,
                    lines: (3, 3),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 1,
                    block_type: BlockType::Body,
                    lines: (4, 7),
                    calls: vec![
                        "input".to_string(),
                        "os.system".to_string(),
                        "eval".to_string(),
                    ],
                },
            ],
            edges: vec![CfgEdge {
                from: 0,
                to: 1,
                edge_type: EdgeType::Unconditional,
                condition: None,
            }],
            entry_block: 0,
            exit_blocks: vec![1],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        };

        let refs = vec![
            VarRef {
                name: "user_input".to_string(),
                ref_type: RefType::Definition,
                line: 3,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "data".to_string(),
                ref_type: RefType::Definition,
                line: 4,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "query".to_string(),
                ref_type: RefType::Definition,
                line: 5,
                column: 0,
                context: None,
                group_id: None,
            },
        ];

        let mut statements: HashMap<u32, String> = HashMap::new();
        for (i, line) in python_code.lines().enumerate() {
            statements.insert((i + 1) as u32, line.to_string());
        }

        let pool = ParserPool::new();
        let tree = pool.parse(python_code, Language::Python).ok();

        let result = compute_taint_with_tree(
            &cfg,
            &refs,
            &statements,
            tree.as_ref(),
            Some(python_code.as_bytes()),
            Language::Python,
        )
        .unwrap();

        // Each unique (line, source_type, var) should appear exactly once
        let mut seen = std::collections::HashSet::new();
        for source in &result.sources {
            let key = (
                source.line,
                std::mem::discriminant(&source.source_type),
                source.var.clone(),
            );
            assert!(
                seen.insert(key.clone()),
                "Duplicate source found: line={}, var={}, type={:?}",
                source.line,
                source.var,
                source.source_type
            );
        }

        // Same for sinks
        let mut seen_sinks = std::collections::HashSet::new();
        for sink in &result.sinks {
            let key = (
                sink.line,
                std::mem::discriminant(&sink.sink_type),
                sink.var.clone(),
            );
            assert!(
                seen_sinks.insert(key.clone()),
                "Duplicate sink found: line={}, var={}, type={:?}",
                sink.line,
                sink.var,
                sink.sink_type
            );
        }
    }

    /// Test that sinks are detected even when AST detection misses them
    /// but regex detection would catch them. Both sources should be merged.
    #[test]
    fn test_sinks_detected_via_merge() {
        use crate::ast::ParserPool;
        use crate::types::{BlockType, CfgBlock, CfgEdge, CfgInfo, EdgeType, RefType, VarRef};

        let python_code = r#"import os

def vuln(user_input):
    os.system(user_input)
    eval(user_input)
"#;

        let cfg = CfgInfo {
            function: "vuln".to_string(),
            blocks: vec![
                CfgBlock {
                    id: 0,
                    block_type: BlockType::Entry,
                    lines: (3, 3),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 1,
                    block_type: BlockType::Body,
                    lines: (4, 5),
                    calls: vec!["os.system".to_string(), "eval".to_string()],
                },
            ],
            edges: vec![CfgEdge {
                from: 0,
                to: 1,
                edge_type: EdgeType::Unconditional,
                condition: None,
            }],
            entry_block: 0,
            exit_blocks: vec![1],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        };

        let refs = vec![VarRef {
            name: "user_input".to_string(),
            ref_type: RefType::Definition,
            line: 3,
            column: 0,
            context: None,
            group_id: None,
        }];

        let mut statements: HashMap<u32, String> = HashMap::new();
        for (i, line) in python_code.lines().enumerate() {
            statements.insert((i + 1) as u32, line.to_string());
        }

        let pool = ParserPool::new();
        let tree = pool.parse(python_code, Language::Python).ok();

        let result = compute_taint_with_tree(
            &cfg,
            &refs,
            &statements,
            tree.as_ref(),
            Some(python_code.as_bytes()),
            Language::Python,
        )
        .unwrap();

        // Should detect at least 2 sinks: os.system and eval
        let sink_types: Vec<_> = result.sinks.iter().map(|s| s.sink_type).collect();
        assert!(
            sink_types.contains(&TaintSinkType::ShellExec),
            "Should detect os.system as ShellExec sink, got: {:?}",
            sink_types
        );
        assert!(
            sink_types.contains(&TaintSinkType::CodeEval),
            "Should detect eval as CodeEval sink, got: {:?}",
            sink_types
        );
    }
}
