//! Vulnerability detection via taint analysis
//!
//! Detects security vulnerabilities by tracking data flow from sources (user input)
//! to sinks (dangerous functions) without proper sanitization.
//!
//! # Vulnerability Types
//!
//! - SQL Injection (CWE-89)
//! - XSS (CWE-79)
//! - Command Injection (CWE-78)
//! - Path Traversal (CWE-22)
//! - SSRF (CWE-918)
//!
//! # TIGER Mitigations
//!
//! - T04: MAX_TAINT_DEPTH = 5 to prevent infinite tracking
//! - Timeout per file analysis
//!
//! # Example
//!
//! ```bash
//! tldr vuln src/
//! tldr vuln app.py --severity critical
//! tldr vuln app.py --vuln-type sql_injection
//! tldr vuln app.py --format sarif
//! ```

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use clap::Args;
use serde_json::{json, Value};
use tldr_core::walker::ProjectWalker;
use tldr_core::Language;
use tree_sitter::{Node, Parser};

use super::error::RemainingError;
use super::types::{Severity, TaintFlow, VulnFinding, VulnReport, VulnSummary, VulnType};
use crate::output::OutputFormat;

// =============================================================================
// Constants - TIGER Mitigations
// =============================================================================

/// Maximum depth for taint propagation (TIGER-04 mitigation)
const MAX_TAINT_DEPTH: usize = 5;

/// Maximum file size to analyze (10 MB).
///
/// Per-file safety cap for the parser: an oversized file can tie up
/// the tree-sitter parser or the line-scanner indefinitely. Unrelated
/// to the total file count (which is bounded structurally by the
/// walker's gitignore + default excludes rather than by a numeric
/// cap — the legacy `MAX_DIRECTORY_FILES = 1000` cap was removed in
/// VAL-006 because it silently truncated input on medium-to-large
/// repos).
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Analyze taint flows to detect security vulnerabilities
///
/// Tracks data flow from sources (user input) to sinks (dangerous functions)
/// to detect injection vulnerabilities like SQL injection, XSS, and command injection.
///
/// # Example
///
/// ```bash
/// tldr vuln src/
/// tldr vuln app.py --severity critical
/// tldr vuln app.py --format sarif
/// ```
#[derive(Debug, Args)]
pub struct VulnArgs {
    /// File or directory to analyze
    pub path: PathBuf,

    /// Programming language to filter by (auto-detected if omitted)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Filter by minimum severity level
    #[arg(long)]
    pub severity: Option<Severity>,

    /// Filter by vulnerability type
    #[arg(long, value_name = "TYPE")]
    pub vuln_type: Option<Vec<VulnType>>,

    /// Include informational findings
    #[arg(long)]
    pub include_informational: bool,

    /// Output file (optional, stdout if not specified)
    #[arg(long, short = 'O')]
    pub output: Option<PathBuf>,

    /// Walk vendored/build dirs (node_modules, target, dist, etc.) that would normally be skipped.
    #[arg(long)]
    pub no_default_ignore: bool,
}

// =============================================================================
// Taint Sources - User Input
// =============================================================================

/// A taint source pattern - where user input enters the program
#[derive(Debug, Clone)]
struct TaintSource {
    /// Module pattern (e.g., "flask", "request")
    module: &'static str,
    /// Function or attribute (e.g., "args", "form", "get")
    attr: &'static str,
    /// Description
    description: &'static str,
}

/// Known taint sources for Python
const PYTHON_SOURCES: &[TaintSource] = &[
    TaintSource {
        module: "request",
        attr: "args",
        description: "Flask request.args (GET parameters)",
    },
    TaintSource {
        module: "request",
        attr: "form",
        description: "Flask request.form (POST data)",
    },
    TaintSource {
        module: "request",
        attr: "get",
        description: "Flask request.get() method",
    },
    TaintSource {
        module: "request",
        attr: "values",
        description: "Flask request.values",
    },
    TaintSource {
        module: "request",
        attr: "data",
        description: "Flask request.data (raw body)",
    },
    TaintSource {
        module: "request",
        attr: "json",
        description: "Flask request.json",
    },
    TaintSource {
        module: "request",
        attr: "cookies",
        description: "Flask request.cookies",
    },
    TaintSource {
        module: "request",
        attr: "headers",
        description: "Flask request.headers",
    },
    TaintSource {
        module: "sys",
        attr: "argv",
        description: "Command line arguments",
    },
    TaintSource {
        module: "",
        attr: "input",
        description: "Python input() builtin",
    },
    TaintSource {
        module: "os",
        attr: "environ",
        description: "Environment variables",
    },
];

// =============================================================================
// Taint Sinks - Dangerous Functions
// =============================================================================

/// A taint sink pattern - dangerous function that should not receive tainted data
#[derive(Debug, Clone)]
struct TaintSink {
    /// Module pattern (e.g., "cursor", "os")
    module: &'static str,
    /// Function name (e.g., "execute", "system")
    function: &'static str,
    /// Vulnerability type
    vuln_type: VulnType,
    /// Description
    description: &'static str,
    /// Remediation advice
    remediation: &'static str,
}

/// Known taint sinks for Python
const PYTHON_SINKS: &[TaintSink] = &[
    // SQL Injection
    TaintSink {
        module: "cursor",
        function: "execute",
        vuln_type: VulnType::SqlInjection,
        description: "SQL query execution with unsanitized input",
        remediation: "Use parameterized queries: cursor.execute(\"SELECT * FROM users WHERE id = ?\", (user_id,))",
    },
    TaintSink {
        module: "cursor",
        function: "executemany",
        vuln_type: VulnType::SqlInjection,
        description: "SQL batch execution with unsanitized input",
        remediation: "Use parameterized queries with placeholders",
    },
    TaintSink {
        module: "",
        function: "raw",
        vuln_type: VulnType::SqlInjection,
        description: "Django raw SQL query",
        remediation: "Use Django ORM methods or parameterized raw queries",
    },
    // Command Injection
    TaintSink {
        module: "os",
        function: "system",
        vuln_type: VulnType::CommandInjection,
        description: "Shell command execution with unsanitized input",
        remediation: "Use subprocess with shell=False and a list of arguments",
    },
    TaintSink {
        module: "os",
        function: "popen",
        vuln_type: VulnType::CommandInjection,
        description: "Shell command via os.popen",
        remediation: "Use subprocess.run with shell=False",
    },
    TaintSink {
        module: "subprocess",
        function: "run",
        vuln_type: VulnType::CommandInjection,
        description: "Subprocess execution (dangerous with shell=True)",
        remediation: "Use subprocess.run with shell=False and pass arguments as a list",
    },
    TaintSink {
        module: "subprocess",
        function: "call",
        vuln_type: VulnType::CommandInjection,
        description: "Subprocess call (dangerous with shell=True)",
        remediation: "Use subprocess.call with shell=False and pass arguments as a list",
    },
    TaintSink {
        module: "subprocess",
        function: "Popen",
        vuln_type: VulnType::CommandInjection,
        description: "Subprocess Popen (dangerous with shell=True)",
        remediation: "Use subprocess.Popen with shell=False",
    },
    TaintSink {
        module: "",
        function: "eval",
        vuln_type: VulnType::CommandInjection,
        description: "Python eval() with user input",
        remediation: "Avoid eval() entirely; use ast.literal_eval() for safe parsing",
    },
    TaintSink {
        module: "",
        function: "exec",
        vuln_type: VulnType::CommandInjection,
        description: "Python exec() with user input",
        remediation: "Avoid exec() entirely; refactor to avoid dynamic code execution",
    },
    // XSS
    TaintSink {
        module: "",
        function: "render_template_string",
        vuln_type: VulnType::Xss,
        description: "Template rendering with unsanitized input",
        remediation: "Use render_template with separate .html files and auto-escaping",
    },
    TaintSink {
        module: "",
        function: "Markup",
        vuln_type: VulnType::Xss,
        description: "Marking string as safe HTML",
        remediation: "Never mark user input as safe; let Jinja2 auto-escape",
    },
    // Path Traversal
    TaintSink {
        module: "",
        function: "open",
        vuln_type: VulnType::PathTraversal,
        description: "File open with user-controlled path",
        remediation: "Validate and sanitize file paths; use os.path.basename()",
    },
    TaintSink {
        module: "os.path",
        function: "join",
        vuln_type: VulnType::PathTraversal,
        description: "Path construction with user input",
        remediation: "Validate that the result is within allowed directories",
    },
    // SSRF
    TaintSink {
        module: "requests",
        function: "get",
        vuln_type: VulnType::Ssrf,
        description: "HTTP request with user-controlled URL",
        remediation: "Validate URLs against an allowlist of permitted hosts",
    },
    TaintSink {
        module: "requests",
        function: "post",
        vuln_type: VulnType::Ssrf,
        description: "HTTP POST with user-controlled URL",
        remediation: "Validate URLs against an allowlist of permitted hosts",
    },
    TaintSink {
        module: "urllib",
        function: "urlopen",
        vuln_type: VulnType::Ssrf,
        description: "URL open with user-controlled input",
        remediation: "Validate URLs against an allowlist",
    },
];

// =============================================================================
// Taint Tracker
// =============================================================================

/// Tracks tainted variables within a function
#[derive(Debug, Default)]
struct TaintTracker {
    /// Variables that are tainted, mapped to their source
    tainted: HashMap<String, TaintInfo>,
    /// Depth of taint propagation (TIGER-04)
    depth: usize,
}

/// Information about a tainted value
#[derive(Debug, Clone)]
struct TaintInfo {
    /// Original source description
    source_desc: String,
    /// Line where taint was introduced
    source_line: u32,
    /// Column where taint was introduced
    source_column: u32,
    /// Code snippet
    code_snippet: String,
}

impl TaintTracker {
    fn new() -> Self {
        Self::default()
    }

    /// Mark a variable as tainted
    fn mark_tainted(&mut self, var: String, info: TaintInfo) {
        self.tainted.insert(var, info);
    }

    #[cfg(test)]
    fn is_tainted(&self, var: &str) -> Option<&TaintInfo> {
        self.tainted.get(var)
    }

    /// Propagate taint from one variable to another
    fn propagate(&mut self, from: &str, to: String) {
        if self.depth >= MAX_TAINT_DEPTH {
            return; // TIGER-04: Stop propagation at max depth
        }
        if let Some(info) = self.tainted.get(from).cloned() {
            self.tainted.insert(to, info);
            self.depth += 1;
        }
    }

    /// Check if an expression contains any tainted variable
    fn expression_is_tainted(&self, expr: &str) -> Option<&TaintInfo> {
        for (var, info) in &self.tainted {
            if expr.contains(var) {
                return Some(info);
            }
        }
        None
    }
}

// =============================================================================
// Implementation
// =============================================================================

impl VulnArgs {
    /// Run the vuln command
    pub fn run(&self, format: OutputFormat) -> Result<()> {
        let start = Instant::now();

        // Validate path exists
        if !self.path.exists() {
            return Err(RemainingError::file_not_found(&self.path).into());
        }

        // Resolve the effective language. When the user passes
        // --lang <L> explicitly, honor it as-is (this is the VAL-001
        // contract: the user knows what they're asking for, even if
        // L lies outside the taint engine's native-analysis set).
        //
        // When --lang is omitted, consult Language::from_directory
        // (the VAL-002 detector: manifest-priority + extension
        // majority, skipping vendored trees). The detector returns
        // Some(L) for any recognised language or None for an empty
        // or unrecognised tree.
        //
        // VAL-006: if the autodetected language lies outside the
        // native-analysis set {Python, Rust}, error out early with
        // exit code 2 and a message that points the user at an
        // explicit --lang flag. This prevents the prior silent
        // behavior where `tldr vuln .` on a TypeScript repo would
        // report "0 files scanned" and exit 0 (misleading).
        //
        // The None case (empty/unrecognised tree) preserves the
        // historical empty-report-exit-0 behavior: the user ran the
        // command with no analyzable input; that's not an error.
        let effective_lang: Option<Language> = match self.lang {
            Some(l) => Some(l),
            None => {
                let detected = if self.path.is_dir() {
                    Language::from_directory(&self.path)
                } else {
                    Language::from_path(&self.path)
                };
                if let Some(l) = detected {
                    if !is_natively_analyzed(l) {
                        return Err(RemainingError::autodetect_unsupported(format!(
                            "vuln: taint analysis for {} is not yet supported by autodetect; \
                             use --lang python or --lang rust to scan files of a supported \
                             language, or omit --lang in a pure Python/Rust project.",
                            l.as_str()
                        ))
                        .into());
                    }
                }
                detected
            }
        };

        // Collect files to analyze
        let files = collect_files(&self.path, effective_lang, self.no_default_ignore)?;

        // Analyze all files
        let mut all_findings: Vec<VulnFinding> = Vec::new();
        let mut files_scanned: u32 = 0;
        let mut files_with_vulns: HashSet<String> = HashSet::new();

        for file_path in &files {
            if let Ok(findings) = analyze_file(file_path) {
                for finding in findings {
                    files_with_vulns.insert(finding.file.clone());
                    all_findings.push(finding);
                }
            }
            files_scanned += 1;
        }

        // Apply filters
        let mut filtered_findings = all_findings;

        // Filter by severity
        if let Some(min_severity) = &self.severity {
            filtered_findings.retain(|f| f.severity.order() <= min_severity.order());
        }

        // Filter by vuln type
        if let Some(types) = &self.vuln_type {
            filtered_findings.retain(|f| types.contains(&f.vuln_type));
        }

        // Filter informational
        if !self.include_informational {
            filtered_findings.retain(|f| f.severity != Severity::Info);
        }

        // Build summary
        let summary = build_summary(&filtered_findings, files_with_vulns.len() as u32);

        // Build report
        let report = VulnReport {
            findings: filtered_findings.clone(),
            summary: Some(summary),
            scan_duration_ms: start.elapsed().as_millis() as u64,
            files_scanned,
        };

        // Output
        let output_str = match format {
            OutputFormat::Sarif => {
                let sarif = generate_sarif(&report);
                serde_json::to_string_pretty(&sarif)?
            }
            OutputFormat::Text => format_vuln_text(&report),
            _ => serde_json::to_string_pretty(&report)?,
        };

        if let Some(ref output_path) = self.output {
            fs::write(output_path, &output_str)?;
        } else {
            println!("{}", output_str);
        }

        // Exit code: 2 if vulnerabilities found (per spec)
        if !filtered_findings.is_empty() {
            return Err(RemainingError::findings_detected(filtered_findings.len() as u32).into());
        }

        Ok(())
    }
}

// =============================================================================
// File Collection
// =============================================================================

/// Collect supported source files to analyze.
fn collect_files(
    path: &Path,
    lang: Option<Language>,
    no_default_ignore: bool,
) -> Result<Vec<PathBuf>, RemainingError> {
    let mut files = Vec::new();

    if path.is_file() {
        // Single file - check size
        let metadata = fs::metadata(path).map_err(|_| RemainingError::file_not_found(path))?;
        if metadata.len() > MAX_FILE_SIZE {
            return Err(RemainingError::file_too_large(path, metadata.len()));
        }
        files.push(path.to_path_buf());
    } else if path.is_dir() {
        // Directory - walk and collect supported source files. The
        // walker is bounded structurally (honors .gitignore and
        // default vendor/build excludes from VAL-001); no numeric
        // file-count cap here, since that silently truncated input
        // on medium-to-large repos (VAL-006).
        let mut walker = ProjectWalker::new(path).max_depth(10);
        if no_default_ignore {
            walker = walker.no_default_ignore();
        }
        for entry in walker.iter() {
            let entry_path = entry.path();
            if entry_path.is_file() && is_supported_source_file(entry_path, lang) {
                // Per-file size cap — an oversized file can stall
                // the parser, but the total file count is not
                // capped.
                if let Ok(metadata) = fs::metadata(entry_path) {
                    if metadata.len() <= MAX_FILE_SIZE {
                        files.push(entry_path.to_path_buf());
                    }
                }
            }
        }
    }

    Ok(files)
}

/// The languages for which the vuln command has a native, dedicated
/// taint-analysis path.
///
/// - Python: tree-sitter-driven intra-procedural taint tracker in
///   `analyze_python_file` + `analyze_node` / `analyze_function`.
/// - Rust: line-scanning unsafe-pattern detector in `analyze_rust_file`.
///
/// Other languages (JS/TS, Go, Java, C, C++, Ruby, Kotlin, Swift,
/// C#, Scala, PHP, Lua/Luau, Elixir, OCaml) fall through to the
/// pattern-based scanner in `tldr_core::security::vuln` — those are
/// meaningful but weaker than the dedicated paths. VAL-006 draws the
/// autodetect-supported set at the native paths so `tldr vuln .`
/// without `--lang` on a non-Python/Rust tree surfaces an explicit
/// error rather than silently delivering weaker analysis.
///
/// An explicit `--lang <L>` bypasses this — the user has signalled
/// they understand which backend will run.
fn is_natively_analyzed(lang: Language) -> bool {
    matches!(lang, Language::Python | Language::Rust)
}

/// Check whether `path` is a source file the vuln scanner should analyze.
///
/// With `lang = Some(L)`, only files matching that language's extensions
/// are accepted. With `lang = None`, we fall back to the historical
/// behavior of `py | rs` (the extensions the taint engine natively
/// supports before multi-language dispatch).
fn is_supported_source_file(path: &Path, lang: Option<Language>) -> bool {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e,
        None => return false,
    };
    match lang {
        Some(Language::TypeScript) => matches!(ext, "ts" | "tsx"),
        Some(Language::JavaScript) => matches!(ext, "js" | "mjs" | "cjs" | "jsx"),
        Some(Language::Python) => ext == "py",
        Some(Language::Rust) => ext == "rs",
        Some(Language::Go) => ext == "go",
        Some(Language::Java) => ext == "java",
        Some(Language::C) => matches!(ext, "c" | "h"),
        Some(Language::Cpp) => matches!(ext, "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx"),
        Some(Language::CSharp) => ext == "cs",
        Some(Language::Ruby) => ext == "rb",
        Some(Language::Php) => ext == "php",
        Some(Language::Kotlin) => matches!(ext, "kt" | "kts"),
        Some(Language::Swift) => ext == "swift",
        Some(Language::Scala) => ext == "scala",
        Some(Language::Elixir) => matches!(ext, "ex" | "exs"),
        Some(Language::Lua) => ext == "lua",
        Some(Language::Luau) => ext == "luau",
        Some(Language::Ocaml) => matches!(ext, "ml" | "mli"),
        // No --lang: preserve historical behavior of scanning py + rs
        // (the two languages the taint analyzer natively handles).
        None => matches!(ext, "py" | "rs"),
    }
}

// =============================================================================
// File Analysis
// =============================================================================

/// Analyze a single file for vulnerabilities
fn analyze_file(path: &Path) -> Result<Vec<VulnFinding>, RemainingError> {
    let source = fs::read_to_string(path).map_err(|_| RemainingError::file_not_found(path))?;
    if matches!(path.extension().and_then(|e| e.to_str()), Some("rs")) {
        return Ok(analyze_rust_file(path, &source));
    }
    if matches!(path.extension().and_then(|e| e.to_str()), Some("py")) {
        return analyze_python_file(path, &source);
    }
    // For all other languages (Go, Java, JS, TS, C, C++, Ruby, PHP, etc.),
    // use tldr-core's multi-language vulnerability scanner
    match tldr_core::security::vuln::scan_vulnerabilities(path, None, None) {
        Ok(report) => {
            let mut findings = Vec::new();
            for f in report.findings {
                let vuln_type = map_core_vuln_type(f.vuln_type);
                let severity = match f.severity.to_uppercase().as_str() {
                    "CRITICAL" => Severity::Critical,
                    "HIGH" => Severity::High,
                    "MEDIUM" => Severity::Medium,
                    "LOW" => Severity::Low,
                    _ => Severity::Medium,
                };
                let file_str = f.file.display().to_string();
                findings.push(VulnFinding {
                    vuln_type,
                    severity,
                    cwe_id: f.cwe_id.unwrap_or_default(),
                    title: format!("{:?}", f.vuln_type),
                    description: format!("{} with unsanitized input", f.sink.sink_type),
                    file: file_str.clone(),
                    line: f.sink.line,
                    column: 0,
                    taint_flow: vec![
                        TaintFlow {
                            file: file_str.clone(),
                            line: f.source.line,
                            column: 0,
                            code_snippet: f.source.expression.clone(),
                            description: format!("Source: {}", f.source.source_type),
                        },
                        TaintFlow {
                            file: file_str,
                            line: f.sink.line,
                            column: 0,
                            code_snippet: f.sink.expression.clone(),
                            description: format!("Sink: {}", f.sink.sink_type),
                        },
                    ],
                    remediation: f.remediation.clone(),
                    confidence: 0.85,
                });
            }
            Ok(findings)
        }
        Err(_) => Ok(Vec::new()),
    }
}

/// Map a `tldr_core::security::vuln::VulnType` to the CLI-side `VulnType`.
///
/// Pre-VAL-002 (issue #11), this site used a wildcard match arm that
/// silently relabeled every variant outside {SqlInjection, CommandInjection,
/// Xss, PathTraversal} as `SqlInjection`. That mislabeled `Deserialization`
/// and `Ssrf` findings — the user-facing symptom in #11 was Java
/// `ObjectInputStream.readObject()` findings being emitted as
/// `vuln_type: "sql_injection"` in JSON, and the SARIF rules array
/// disagreeing with `results[].ruleId` (rules: `CWE-89` from local
/// vuln_type, results.ruleId: `CWE-502` from the unmodified `cwe_id`),
/// producing an internally inconsistent SARIF document.
///
/// This match is deliberately exhaustive (no `_` arm). When tldr-core
/// adds a new `VulnType` variant in the future, this function fails
/// to compile until the new variant is mapped — preventing a
/// reintroduction of the wildcard mislabel.
fn map_core_vuln_type(core_ty: tldr_core::security::vuln::VulnType) -> VulnType {
    use tldr_core::security::vuln::VulnType as CoreVulnType;
    match core_ty {
        CoreVulnType::SqlInjection => VulnType::SqlInjection,
        CoreVulnType::Xss => VulnType::Xss,
        CoreVulnType::CommandInjection => VulnType::CommandInjection,
        CoreVulnType::PathTraversal => VulnType::PathTraversal,
        CoreVulnType::Ssrf => VulnType::Ssrf,
        CoreVulnType::Deserialization => VulnType::Deserialization,
    }
}

fn analyze_python_file(path: &Path, source: &str) -> Result<Vec<VulnFinding>, RemainingError> {
    // Parse with tree-sitter
    let mut parser = get_python_parser()?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| RemainingError::parse_error(path, "Failed to parse file"))?;

    let mut findings = Vec::new();
    let source_bytes = source.as_bytes();
    let file_path = path.display().to_string();

    // Analyze each function
    analyze_node(tree.root_node(), source_bytes, &file_path, &mut findings);

    Ok(findings)
}

fn analyze_rust_file(path: &Path, source: &str) -> Vec<VulnFinding> {
    let file_path = path.display().to_string();
    let is_test_file = is_rust_test_file(path);
    let mut findings = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut in_command_block = false;
    let mut command_block_start_line: u32 = 0;

    for (idx, line) in lines.iter().enumerate() {
        let line_number = (idx + 1) as u32;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        if (trimmed.contains("unsafe {") || trimmed.starts_with("unsafe{"))
            && !has_nearby_safety_comment(&lines, idx)
        {
            findings.push(rust_finding(
                VulnType::UnsafeCode,
                Severity::High,
                RustFindingMeta {
                    cwe_id: "CWE-242",
                    title: "Unsafe Block Without Safety Rationale",
                    description: "unsafe block found without nearby SAFETY: justification comment",
                },
                RustFindingLocation {
                    file: &file_path,
                    line: line_number,
                    column: trimmed.find("unsafe").unwrap_or(0) as u32,
                },
                "Document invariants with // SAFETY: ... or avoid unsafe when possible",
                0.80,
            ));
        }

        if trimmed.contains("std::mem::transmute(") || trimmed.contains("mem::transmute(") {
            findings.push(rust_finding(
                VulnType::MemorySafety,
                Severity::Critical,
                RustFindingMeta {
                    cwe_id: "CWE-119",
                    title: "Risky transmute Usage",
                    description:
                        "std::mem::transmute can violate type and memory safety guarantees",
                },
                RustFindingLocation {
                    file: &file_path,
                    line: line_number,
                    column: trimmed.find("transmute").unwrap_or(0) as u32,
                },
                "Prefer safe conversions (From/TryFrom, bytemuck) and explicit layout checks",
                0.90,
            ));
        }

        if trimmed.contains("std::ptr::")
            || trimmed.contains("core::ptr::")
            || trimmed.contains("ptr::read(")
            || trimmed.contains("ptr::write(")
        {
            findings.push(rust_finding(
                VulnType::MemorySafety,
                Severity::High,
                RustFindingMeta {
                    cwe_id: "CWE-119",
                    title: "Raw Pointer Operation",
                    description:
                        "raw pointer operation detected; verify lifetimes, aliasing, and bounds",
                },
                RustFindingLocation {
                    file: &file_path,
                    line: line_number,
                    column: trimmed.find("ptr::").unwrap_or(0) as u32,
                },
                "Use safe abstractions/slices where possible and document pointer invariants",
                0.85,
            ));
        }

        if !is_test_file && trimmed.contains(".unwrap()") {
            findings.push(rust_finding(
                VulnType::Panic,
                Severity::Medium,
                RustFindingMeta {
                    cwe_id: "CWE-703",
                    title: "Potential Panic From unwrap()",
                    description: "unwrap() in non-test Rust code can panic in production paths",
                },
                RustFindingLocation {
                    file: &file_path,
                    line: line_number,
                    column: trimmed.find(".unwrap()").unwrap_or(0) as u32,
                },
                "Handle Result/Option explicitly or use expect() with actionable context",
                0.70,
            ));
        }

        if trimmed.contains("format!(")
            && contains_sql_keyword(trimmed)
            && (trimmed.contains("{}") || trimmed.contains("{") || trimmed.contains("+"))
        {
            findings.push(rust_finding(
                VulnType::SqlInjection,
                Severity::Critical,
                RustFindingMeta {
                    cwe_id: "CWE-89",
                    title: "SQL String Interpolation",
                    description:
                        "SQL query appears to be built via string formatting/interpolation",
                },
                RustFindingLocation {
                    file: &file_path,
                    line: line_number,
                    column: trimmed.find("format!(").unwrap_or(0) as u32,
                },
                "Use parameterized queries via your DB client instead of format!/concatenation",
                0.88,
            ));
        }

        if trimmed.contains("from_utf8_unchecked(")
            || trimmed.contains(".as_bytes()[")
            || trimmed.contains(".as_bytes().get_unchecked(")
        {
            findings.push(rust_finding(
                VulnType::MemorySafety,
                Severity::High,
                RustFindingMeta {
                    cwe_id: "CWE-20",
                    title: "Unchecked Byte/String Conversion",
                    description:
                        "unchecked UTF-8 or byte indexing detected without visible validation",
                },
                RustFindingLocation {
                    file: &file_path,
                    line: line_number,
                    column: trimmed
                        .find("as_bytes")
                        .or_else(|| trimmed.find("from_utf8_unchecked"))
                        .unwrap_or(0) as u32,
                },
                "Validate lengths/UTF-8 before conversion or use checked APIs",
                0.82,
            ));
        }

        if trimmed.contains("Command::new(") || trimmed.contains("std::process::Command::new(") {
            in_command_block = true;
            command_block_start_line = line_number;
        }
        if in_command_block
            && trimmed.contains(".arg(")
            && !trimmed.contains(".arg(\"")
            && !trimmed.contains(".arg('")
        {
            findings.push(rust_finding(
                VulnType::CommandInjection,
                Severity::Critical,
                RustFindingMeta {
                    cwe_id: "CWE-78",
                    title: "Unsanitized Process Argument",
                    description: "Command argument appears to be variable-driven without visible sanitization",
                },
                RustFindingLocation {
                    file: &file_path,
                    line: command_block_start_line.max(line_number),
                    column: trimmed.find(".arg(").unwrap_or(0) as u32,
                },
                "Validate/allowlist user-controlled arguments before passing to Command",
                0.80,
            ));
        }
        if in_command_block && (trimmed.ends_with(';') || trimmed.contains(");")) {
            in_command_block = false;
            command_block_start_line = 0;
        }
    }

    findings
}

struct RustFindingMeta<'a> {
    cwe_id: &'a str,
    title: &'a str,
    description: &'a str,
}

struct RustFindingLocation<'a> {
    file: &'a str,
    line: u32,
    column: u32,
}

fn rust_finding(
    vuln_type: VulnType,
    severity: Severity,
    meta: RustFindingMeta<'_>,
    location: RustFindingLocation<'_>,
    remediation: &str,
    confidence: f64,
) -> VulnFinding {
    VulnFinding {
        vuln_type,
        severity,
        cwe_id: meta.cwe_id.to_string(),
        title: meta.title.to_string(),
        description: meta.description.to_string(),
        file: location.file.to_string(),
        line: location.line,
        column: location.column,
        taint_flow: Vec::new(),
        remediation: remediation.to_string(),
        confidence,
    }
}

fn has_nearby_safety_comment(lines: &[&str], index: usize) -> bool {
    let start = index.saturating_sub(2);
    (start..index).any(|i| lines[i].contains("SAFETY:"))
}

fn contains_sql_keyword(text: &str) -> bool {
    let upper = text.to_uppercase();
    ["SELECT", "INSERT", "UPDATE", "DELETE", "FROM", "WHERE"]
        .iter()
        .any(|kw| upper.contains(kw))
}

fn is_rust_test_file(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains("/tests/")
        || path_str.contains("\\tests\\")
        || path_str.ends_with("_test.rs")
        || path_str.ends_with("tests.rs")
}

/// Recursively analyze AST nodes
fn analyze_node(node: Node, source: &[u8], file_path: &str, findings: &mut Vec<VulnFinding>) {
    match node.kind() {
        "function_definition" | "async_function_definition" => {
            analyze_function(node, source, file_path, findings);
        }
        "decorated_definition" => {
            // Handle decorated functions - the actual function is a child
            for child in node.children(&mut node.walk()) {
                if child.kind() == "function_definition"
                    || child.kind() == "async_function_definition"
                {
                    analyze_function(child, source, file_path, findings);
                }
            }
        }
        _ => {
            // Recurse into children
            for child in node.children(&mut node.walk()) {
                analyze_node(child, source, file_path, findings);
            }
        }
    }
}

/// Analyze a function for taint vulnerabilities
fn analyze_function(
    func_node: Node,
    source: &[u8],
    file_path: &str,
    findings: &mut Vec<VulnFinding>,
) {
    let mut tracker = TaintTracker::new();
    let source_lines: Vec<&str> = std::str::from_utf8(source).unwrap_or("").lines().collect();

    // Find function body
    if let Some(body) = func_node.child_by_field_name("body") {
        analyze_block(
            body,
            source,
            file_path,
            &mut tracker,
            findings,
            &source_lines,
        );
    }
}

/// Analyze a block of statements
fn analyze_block(
    block: Node,
    source: &[u8],
    file_path: &str,
    tracker: &mut TaintTracker,
    findings: &mut Vec<VulnFinding>,
    source_lines: &[&str],
) {
    for child in block.children(&mut block.walk()) {
        analyze_statement(child, source, file_path, tracker, findings, source_lines);
    }
}

/// Analyze a single statement
fn analyze_statement(
    stmt: Node,
    source: &[u8],
    file_path: &str,
    tracker: &mut TaintTracker,
    findings: &mut Vec<VulnFinding>,
    source_lines: &[&str],
) {
    match stmt.kind() {
        "expression_statement" => {
            if let Some(expr) = stmt.child(0) {
                // Handle assignments inside expression_statement
                if expr.kind() == "assignment" {
                    analyze_assignment(expr, source, tracker, source_lines);
                }
                analyze_expression(expr, source, file_path, tracker, findings, source_lines);
            }
        }
        "assignment" => {
            analyze_assignment(stmt, source, tracker, source_lines);
        }
        "augmented_assignment" => {
            analyze_augmented_assignment(stmt, source, tracker);
        }
        "if_statement" | "for_statement" | "while_statement" | "with_statement" => {
            // Recurse into nested blocks
            for child in stmt.children(&mut stmt.walk()) {
                if child.kind() == "block" {
                    analyze_block(child, source, file_path, tracker, findings, source_lines);
                }
            }
        }
        "return_statement" => {
            // Check if returning tainted data (potential XSS in web context)
            if let Some(value) = stmt.child_by_field_name("value").or_else(|| stmt.child(1)) {
                let value_text = node_text(value, source);
                check_xss_return(
                    value_text,
                    value,
                    file_path,
                    tracker,
                    findings,
                    source_lines,
                );
            }
        }
        _ => {
            // Recurse into children for other statement types
            for child in stmt.children(&mut stmt.walk()) {
                analyze_statement(child, source, file_path, tracker, findings, source_lines);
            }
        }
    }
}

/// Analyze an assignment statement
fn analyze_assignment(
    assignment: Node,
    source: &[u8],
    tracker: &mut TaintTracker,
    source_lines: &[&str],
) {
    // Get LHS (target)
    let lhs = assignment
        .child_by_field_name("left")
        .or_else(|| assignment.child(0));
    // Get RHS (value)
    let rhs = assignment
        .child_by_field_name("right")
        .or_else(|| assignment.child(2));

    if let (Some(lhs_node), Some(rhs_node)) = (lhs, rhs) {
        let lhs_text = node_text(lhs_node, source);
        let rhs_text = node_text(rhs_node, source);
        let line = rhs_node.start_position().row as u32 + 1;
        let column = rhs_node.start_position().column as u32;

        // Check if RHS is a taint source
        if let Some(source_desc) = is_taint_source(rhs_text) {
            let code_snippet = source_lines
                .get(line as usize - 1)
                .map(|s| s.to_string())
                .unwrap_or_default();
            tracker.mark_tainted(
                lhs_text.to_string(),
                TaintInfo {
                    source_desc,
                    source_line: line,
                    source_column: column,
                    code_snippet,
                },
            );
        }

        // Check if RHS contains a tainted variable (propagation)
        if let Some(taint_info) = tracker.expression_is_tainted(rhs_text) {
            let code_snippet = source_lines
                .get(line as usize - 1)
                .map(|s| s.to_string())
                .unwrap_or_default();
            tracker.mark_tainted(
                lhs_text.to_string(),
                TaintInfo {
                    source_desc: taint_info.source_desc.clone(),
                    source_line: line,
                    source_column: column,
                    code_snippet,
                },
            );
        }
    }
}

/// Analyze augmented assignment (+=, etc.)
fn analyze_augmented_assignment(assignment: Node, source: &[u8], tracker: &mut TaintTracker) {
    // For augmented assignment, if RHS is tainted, LHS becomes tainted
    let lhs = assignment
        .child_by_field_name("left")
        .or_else(|| assignment.child(0));
    let rhs = assignment
        .child_by_field_name("right")
        .or_else(|| assignment.child(2));

    if let (Some(lhs_node), Some(rhs_node)) = (lhs, rhs) {
        let lhs_text = node_text(lhs_node, source);
        let rhs_text = node_text(rhs_node, source);

        // Propagate taint
        if tracker.expression_is_tainted(rhs_text).is_some() {
            tracker.propagate(rhs_text, lhs_text.to_string());
        }
    }
}

/// Analyze an expression (look for sink calls)
fn analyze_expression(
    expr: Node,
    source: &[u8],
    file_path: &str,
    tracker: &mut TaintTracker,
    findings: &mut Vec<VulnFinding>,
    source_lines: &[&str],
) {
    match expr.kind() {
        "call" => {
            analyze_call(expr, source, file_path, tracker, findings, source_lines);
        }
        _ => {
            // Recurse into children
            for child in expr.children(&mut expr.walk()) {
                analyze_expression(child, source, file_path, tracker, findings, source_lines);
            }
        }
    }
}

/// Analyze a function call - check if it's a sink with tainted arguments
fn analyze_call(
    call: Node,
    source: &[u8],
    file_path: &str,
    tracker: &mut TaintTracker,
    findings: &mut Vec<VulnFinding>,
    source_lines: &[&str],
) {
    // Get function being called
    let func = call
        .child_by_field_name("function")
        .or_else(|| call.child(0));
    let args = call.child_by_field_name("arguments");

    if let Some(func_node) = func {
        let func_text = node_text(func_node, source);
        let line = call.start_position().row as u32 + 1;
        let column = call.start_position().column as u32;

        // Check if this is a known sink
        if let Some(sink) = is_taint_sink(func_text) {
            // Check if any arguments are tainted
            if let Some(args_node) = args {
                let args_text = node_text(args_node, source);

                // Check for parameterized query (safe pattern)
                // e.g., cursor.execute("SELECT * FROM users WHERE id = ?", (user_id,))
                if sink.vuln_type == VulnType::SqlInjection && is_parameterized_query(args_text) {
                    // This is a parameterized query - it's safe even with tainted data
                    // The tainted data goes into the parameters tuple, not the query string
                    return; // Skip this call, it's not a vulnerability
                }

                // Check for direct tainted variable
                if let Some(taint_info) = tracker.expression_is_tainted(args_text) {
                    let code_snippet = source_lines
                        .get(line as usize - 1)
                        .map(|s| s.to_string())
                        .unwrap_or_default();

                    // Build taint flow
                    let taint_flow = vec![
                        TaintFlow {
                            file: file_path.to_string(),
                            line: taint_info.source_line,
                            column: taint_info.source_column,
                            code_snippet: taint_info.code_snippet.clone(),
                            description: format!("Source: {}", taint_info.source_desc),
                        },
                        TaintFlow {
                            file: file_path.to_string(),
                            line,
                            column,
                            code_snippet: code_snippet.clone(),
                            description: format!("Sink: {} call", func_text),
                        },
                    ];

                    findings.push(VulnFinding {
                        vuln_type: sink.vuln_type,
                        severity: sink.vuln_type.default_severity(),
                        cwe_id: sink.vuln_type.cwe_id().to_string(),
                        title: format!("{} Vulnerability", vuln_type_name(sink.vuln_type)),
                        description: sink.description.to_string(),
                        file: file_path.to_string(),
                        line,
                        column,
                        taint_flow,
                        remediation: sink.remediation.to_string(),
                        confidence: 0.85,
                    });
                }

                // Also check for f-string or string concatenation with tainted var
                if is_string_interpolation_tainted(args_text, tracker) {
                    let code_snippet = source_lines
                        .get(line as usize - 1)
                        .map(|s| s.to_string())
                        .unwrap_or_default();

                    // Find the tainted variable for flow
                    let taint_info = find_taint_in_string(args_text, tracker);

                    let taint_flow = if let Some(info) = taint_info {
                        vec![
                            TaintFlow {
                                file: file_path.to_string(),
                                line: info.source_line,
                                column: info.source_column,
                                code_snippet: info.code_snippet.clone(),
                                description: format!("Source: {}", info.source_desc),
                            },
                            TaintFlow {
                                file: file_path.to_string(),
                                line,
                                column,
                                code_snippet: code_snippet.clone(),
                                description: format!(
                                    "Sink: {} call with string interpolation",
                                    func_text
                                ),
                            },
                        ]
                    } else {
                        vec![TaintFlow {
                            file: file_path.to_string(),
                            line,
                            column,
                            code_snippet,
                            description: format!("Sink: {} call", func_text),
                        }]
                    };

                    findings.push(VulnFinding {
                        vuln_type: sink.vuln_type,
                        severity: sink.vuln_type.default_severity(),
                        cwe_id: sink.vuln_type.cwe_id().to_string(),
                        title: format!("{} Vulnerability", vuln_type_name(sink.vuln_type)),
                        description: sink.description.to_string(),
                        file: file_path.to_string(),
                        line,
                        column,
                        taint_flow,
                        remediation: sink.remediation.to_string(),
                        confidence: 0.8,
                    });
                }
            }
        }

        // Check for shell=True in subprocess calls
        if func_text.contains("subprocess")
            || func_text == "run"
            || func_text == "call"
            || func_text == "Popen"
        {
            if let Some(args_node) = args {
                let args_text = node_text(args_node, source);
                if args_text.contains("shell=True") || args_text.contains("shell = True") {
                    if let Some(taint_info) = tracker.expression_is_tainted(args_text) {
                        let code_snippet = source_lines
                            .get(line as usize - 1)
                            .map(|s| s.to_string())
                            .unwrap_or_default();

                        let taint_flow = vec![
                            TaintFlow {
                                file: file_path.to_string(),
                                line: taint_info.source_line,
                                column: taint_info.source_column,
                                code_snippet: taint_info.code_snippet.clone(),
                                description: format!("Source: {}", taint_info.source_desc),
                            },
                            TaintFlow {
                                file: file_path.to_string(),
                                line,
                                column,
                                code_snippet,
                                description: "Sink: subprocess with shell=True".to_string(),
                            },
                        ];

                        findings.push(VulnFinding {
                            vuln_type: VulnType::CommandInjection,
                            severity: Severity::Critical,
                            cwe_id: "CWE-78".to_string(),
                            title: "Command Injection Vulnerability".to_string(),
                            description:
                                "Subprocess executed with shell=True and user-controlled input"
                                    .to_string(),
                            file: file_path.to_string(),
                            line,
                            column,
                            taint_flow,
                            remediation:
                                "Use subprocess.run with shell=False and pass arguments as a list"
                                    .to_string(),
                            confidence: 0.9,
                        });
                    }
                }
            }
        }
    }
}

/// Check for XSS in return statements (returning tainted data directly)
fn check_xss_return(
    value_text: &str,
    value_node: Node,
    file_path: &str,
    tracker: &mut TaintTracker,
    findings: &mut Vec<VulnFinding>,
    source_lines: &[&str],
) {
    // Check for f-string returns with tainted data (common XSS pattern)
    if value_text.starts_with("f\"") || value_text.starts_with("f'") {
        if let Some(taint_info) = find_taint_in_string(value_text, tracker) {
            let line = value_node.start_position().row as u32 + 1;
            let column = value_node.start_position().column as u32;
            let code_snippet = source_lines
                .get(line as usize - 1)
                .map(|s| s.to_string())
                .unwrap_or_default();

            // Only flag as XSS if it looks like HTML
            if value_text.contains('<') && value_text.contains('>') {
                let taint_flow = vec![
                    TaintFlow {
                        file: file_path.to_string(),
                        line: taint_info.source_line,
                        column: taint_info.source_column,
                        code_snippet: taint_info.code_snippet.clone(),
                        description: format!("Source: {}", taint_info.source_desc),
                    },
                    TaintFlow {
                        file: file_path.to_string(),
                        line,
                        column,
                        code_snippet,
                        description: "Sink: Returning HTML with user input".to_string(),
                    },
                ];

                findings.push(VulnFinding {
                    vuln_type: VulnType::Xss,
                    severity: Severity::High,
                    cwe_id: "CWE-79".to_string(),
                    title: "Cross-Site Scripting (XSS) Vulnerability".to_string(),
                    description: "User input embedded in HTML response without escaping"
                        .to_string(),
                    file: file_path.to_string(),
                    line,
                    column,
                    taint_flow,
                    remediation: "Use a templating engine with auto-escaping or escape user input"
                        .to_string(),
                    confidence: 0.75,
                });
            }
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Initialize tree-sitter parser for Python
fn get_python_parser() -> Result<Parser, RemainingError> {
    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE;
    parser.set_language(&language.into()).map_err(|e| {
        RemainingError::parse_error(PathBuf::new(), format!("Failed to set language: {}", e))
    })?;
    Ok(parser)
}

/// Get text for a node from source
fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Check if an expression is a taint source
fn is_taint_source(expr: &str) -> Option<String> {
    for source in PYTHON_SOURCES {
        // Check for attribute access patterns
        if !source.module.is_empty() {
            let pattern = format!("{}.{}", source.module, source.attr);
            if expr.contains(&pattern) {
                return Some(source.description.to_string());
            }
        } else {
            // Builtin function
            if expr.contains(&format!("{}(", source.attr)) {
                return Some(source.description.to_string());
            }
        }

        // Also check for method call pattern: request.args.get(...)
        if expr.contains(&format!(".{}.get", source.attr))
            || expr.contains(&format!(".{}[", source.attr))
        {
            return Some(source.description.to_string());
        }
    }
    None
}

/// Check if an expression matches a taint sink
fn is_taint_sink(func_expr: &str) -> Option<&'static TaintSink> {
    // Method names that are common Python builtins/dict methods.
    // These must NOT match module-specific sinks via suffix alone
    // (e.g., dict.get() is not requests.get() SSRF).
    const AMBIGUOUS_METHODS: &[&str] = &["get", "post", "put", "delete", "read", "write", "open"];

    for sink in PYTHON_SINKS {
        if !sink.module.is_empty() {
            // Check for module.function pattern (exact)
            let pattern = format!("{}.{}", sink.module, sink.function);
            if func_expr.contains(&pattern) {
                return Some(sink);
            }
            // Suffix matching (.function()) — but skip ambiguous method names
            // to avoid false positives like body.get() matching requests.get SSRF
            if !AMBIGUOUS_METHODS.contains(&sink.function)
                && func_expr.ends_with(&format!(".{}", sink.function))
            {
                return Some(sink);
            }
        } else {
            // Check for standalone function
            if func_expr == sink.function || func_expr.ends_with(&format!(".{}", sink.function)) {
                return Some(sink);
            }
        }
    }
    None
}

/// Check if arguments represent a parameterized SQL query (safe pattern)
/// Parameterized queries use ? or %s placeholders and pass values separately
fn is_parameterized_query(args_text: &str) -> bool {
    // Look for patterns like:
    // ("SELECT * FROM users WHERE id = ?", (user_id,))
    // ("SELECT * FROM users WHERE id = %s", (user_id,))
    // "SELECT * FROM users WHERE id = ?", [user_id]

    // Must have a query string with placeholder and a separate params argument
    let has_placeholder = args_text.contains("?")
        || args_text.contains("%s")
        || args_text.contains(":param")
        || args_text.contains("$1");

    // Must have a tuple or list for parameters (second argument)
    let has_params_collection = args_text.contains(", (")
        || args_text.contains(", [")
        || args_text.contains(",(")
        || args_text.contains(",[");

    // If query has placeholder AND has params, it's parameterized
    // Also check that the query string itself doesn't use f-string or concatenation
    // Query string is typically the first argument - we need the first quoted string
    // to NOT be an f-string and NOT contain concatenation before the comma

    if has_placeholder && has_params_collection {
        // Find where the first string ends (before the params tuple)
        // Check if the query string uses f-string or concatenation
        if let Some(comma_pos) = args_text.find(", (").or_else(|| args_text.find(",(")) {
            let query_part = &args_text[..comma_pos];
            // If query part has f-string or concatenation with variables, it's NOT parameterized
            let is_unsafe = query_part.contains("f\"")
                || query_part.contains("f'")
                || (query_part.contains(" + ")
                    && !query_part.trim_start().starts_with("(\"")
                    && !query_part.trim_start().starts_with("('"));
            return !is_unsafe;
        }
        return true;
    }

    false
}

/// Check if a string contains interpolation with tainted variables
fn is_string_interpolation_tainted(text: &str, tracker: &TaintTracker) -> bool {
    // Check f-strings
    if text.contains("f\"") || text.contains("f'") {
        for var in tracker.tainted.keys() {
            // Check for {var} or {{var}} patterns in f-strings
            let pattern1 = format!("{{{}}}", var); // {var}
            let pattern2 = format!("{{{{{}}}}}", var); // {{var}}
            if text.contains(&pattern1) || text.contains(&pattern2) {
                return true;
            }
        }
    }

    // Check string concatenation
    if text.contains(" + ") || text.contains("\" +") || text.contains("' +") {
        for var in tracker.tainted.keys() {
            if text.contains(var) {
                return true;
            }
        }
    }

    // Check % formatting
    if text.contains(" % ") {
        for var in tracker.tainted.keys() {
            if text.contains(var) {
                return true;
            }
        }
    }

    // Check .format()
    if text.contains(".format(") {
        for var in tracker.tainted.keys() {
            if text.contains(var) {
                return true;
            }
        }
    }

    false
}

/// Find taint info for a variable in a string expression
fn find_taint_in_string<'a>(text: &str, tracker: &'a TaintTracker) -> Option<&'a TaintInfo> {
    for (var, info) in &tracker.tainted {
        if text.contains(var) {
            return Some(info);
        }
    }
    None
}

/// Get human-readable name for vulnerability type
fn vuln_type_name(vt: VulnType) -> &'static str {
    match vt {
        VulnType::SqlInjection => "SQL Injection",
        VulnType::Xss => "Cross-Site Scripting (XSS)",
        VulnType::CommandInjection => "Command Injection",
        VulnType::Ssrf => "Server-Side Request Forgery (SSRF)",
        VulnType::PathTraversal => "Path Traversal",
        VulnType::Deserialization => "Insecure Deserialization",
        VulnType::UnsafeCode => "Unsafe Code Risk",
        VulnType::MemorySafety => "Memory Safety Violation",
        VulnType::Panic => "Unchecked Panic Path",
        VulnType::Xxe => "XML External Entity (XXE)",
        VulnType::OpenRedirect => "Open Redirect",
        VulnType::LdapInjection => "LDAP Injection",
        VulnType::XpathInjection => "XPath Injection",
    }
}

/// Build summary statistics
fn build_summary(findings: &[VulnFinding], files_with_vulns: u32) -> VulnSummary {
    let mut by_severity: HashMap<String, u32> = HashMap::new();
    let mut by_type: HashMap<String, u32> = HashMap::new();

    for finding in findings {
        *by_severity.entry(finding.severity.to_string()).or_insert(0) += 1;
        *by_type
            .entry(format!("{:?}", finding.vuln_type).to_lowercase())
            .or_insert(0) += 1;
    }

    VulnSummary {
        total_findings: findings.len() as u32,
        by_severity,
        by_type,
        files_with_vulns,
    }
}

// =============================================================================
// Output Formatting
// =============================================================================

/// Format report as human-readable text
fn format_vuln_text(report: &VulnReport) -> String {
    let mut out = String::new();

    out.push_str("=== Vulnerability Scan Results ===\n\n");

    if report.findings.is_empty() {
        out.push_str("No vulnerabilities found.\n");
    } else {
        out.push_str(&format!(
            "Found {} vulnerabilities:\n\n",
            report.findings.len()
        ));

        for (i, finding) in report.findings.iter().enumerate() {
            out.push_str(&format!(
                "{}. [{}] {} ({})\n",
                i + 1,
                finding.severity.to_string().to_uppercase(),
                finding.title,
                finding.cwe_id
            ));
            out.push_str(&format!("   File: {}:{}\n", finding.file, finding.line));
            out.push_str(&format!("   {}\n", finding.description));

            if !finding.taint_flow.is_empty() {
                out.push_str("   Taint Flow:\n");
                for (j, flow) in finding.taint_flow.iter().enumerate() {
                    out.push_str(&format!(
                        "     {}. {}:{} - {}\n",
                        j + 1,
                        flow.file,
                        flow.line,
                        flow.description
                    ));
                    if !flow.code_snippet.is_empty() {
                        out.push_str(&format!("        {}\n", flow.code_snippet.trim()));
                    }
                }
            }

            out.push_str(&format!("   Remediation: {}\n\n", finding.remediation));
        }
    }

    if let Some(summary) = &report.summary {
        out.push_str("=== Summary ===\n");
        out.push_str(&format!(
            "Total: {} vulnerabilities\n",
            summary.total_findings
        ));
        out.push_str(&format!(
            "Files with vulnerabilities: {}\n",
            summary.files_with_vulns
        ));

        if !summary.by_severity.is_empty() {
            out.push_str("By Severity:\n");
            for (sev, count) in &summary.by_severity {
                out.push_str(&format!("  {}: {}\n", sev, count));
            }
        }
    }

    out.push_str(&format!("\nScan duration: {}ms\n", report.scan_duration_ms));
    out.push_str(&format!("Files scanned: {}\n", report.files_scanned));

    out
}

/// Generate SARIF format output
fn generate_sarif(report: &VulnReport) -> Value {
    let results: Vec<Value> = report
        .findings
        .iter()
        .map(|f| {
            json!({
                "ruleId": f.cwe_id,
                "level": match f.severity {
                    Severity::Critical | Severity::High => "error",
                    Severity::Medium => "warning",
                    Severity::Low | Severity::Info => "note",
                },
                "message": {
                    "text": f.description
                },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": {
                            "uri": f.file
                        },
                        "region": {
                            "startLine": f.line,
                            "startColumn": f.column
                        }
                    }
                }],
                "codeFlows": if f.taint_flow.is_empty() { None } else {
                    Some(vec![{
                        json!({
                            "threadFlows": [{
                                "locations": f.taint_flow.iter().map(|tf| {
                                    json!({
                                        "location": {
                                            "physicalLocation": {
                                                "artifactLocation": {
                                                    "uri": tf.file
                                                },
                                                "region": {
                                                    "startLine": tf.line,
                                                    "startColumn": tf.column
                                                }
                                            },
                                            "message": {
                                                "text": tf.description
                                            }
                                        }
                                    })
                                }).collect::<Vec<_>>()
                            }]
                        })
                    }])
                }
            })
        })
        .collect();

    let rules: Vec<Value> = report
        .findings
        .iter()
        .map(|f| &f.vuln_type)
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|vt| {
            json!({
                "id": vt.cwe_id(),
                "name": vuln_type_name(*vt),
                "shortDescription": {
                    "text": vuln_type_name(*vt)
                },
                "defaultConfiguration": {
                    "level": match vt.default_severity() {
                        Severity::Critical | Severity::High => "error",
                        Severity::Medium => "warning",
                        Severity::Low | Severity::Info => "note",
                    }
                }
            })
        })
        .collect();

    json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "tldr-vuln",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/tldr-code/tldr-rs",
                    "rules": rules
                }
            },
            "results": results
        }]
    })
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_is_taint_source() {
        assert!(is_taint_source("request.args.get('q')").is_some());
        assert!(is_taint_source("request.form").is_some());
        assert!(is_taint_source("input()").is_some());
        assert!(is_taint_source("sys.argv").is_some());
        assert!(is_taint_source("clean_var").is_none());
    }

    #[test]
    fn test_is_taint_sink() {
        assert!(is_taint_sink("cursor.execute").is_some());
        assert!(is_taint_sink("os.system").is_some());
        assert!(is_taint_sink("eval").is_some());
        assert!(is_taint_sink("print").is_none());
    }

    #[test]
    fn test_taint_tracker() {
        let mut tracker = TaintTracker::new();

        tracker.mark_tainted(
            "user_input".to_string(),
            TaintInfo {
                source_desc: "request.args".to_string(),
                source_line: 5,
                source_column: 0,
                code_snippet: "user_input = request.args.get('q')".to_string(),
            },
        );

        assert!(tracker.is_tainted("user_input").is_some());
        assert!(tracker.is_tainted("clean_var").is_none());
        assert!(tracker
            .expression_is_tainted("f\"SELECT * FROM t WHERE x = {user_input}\"")
            .is_some());
    }

    #[test]
    fn test_vuln_type_cwe_mapping() {
        assert_eq!(VulnType::SqlInjection.cwe_id(), "CWE-89");
        assert_eq!(VulnType::Xss.cwe_id(), "CWE-79");
        assert_eq!(VulnType::CommandInjection.cwe_id(), "CWE-78");
    }

    #[test]
    fn test_vuln_type_severity() {
        assert_eq!(
            VulnType::SqlInjection.default_severity(),
            Severity::Critical
        );
        assert_eq!(VulnType::Xss.default_severity(), Severity::High);
        assert_eq!(VulnType::OpenRedirect.default_severity(), Severity::Medium);
    }

    #[test]
    fn test_string_interpolation_detection() {
        let mut tracker = TaintTracker::new();
        tracker.mark_tainted(
            "user_query".to_string(),
            TaintInfo {
                source_desc: "test".to_string(),
                source_line: 1,
                source_column: 0,
                code_snippet: "".to_string(),
            },
        );

        assert!(is_string_interpolation_tainted(
            r#"f"SELECT * FROM t WHERE x = '{user_query}'"#,
            &tracker
        ));
        assert!(is_string_interpolation_tainted(
            r#""SELECT * FROM t WHERE x = '" + user_query + "'"#,
            &tracker
        ));
        assert!(!is_string_interpolation_tainted(
            r#""SELECT * FROM t WHERE x = ?""#,
            &tracker
        ));
    }

    #[test]
    fn test_collect_files_includes_rust() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("a.py"), "print('ok')").unwrap();
        std::fs::write(temp.path().join("b.rs"), "fn main() {}").unwrap();
        std::fs::write(temp.path().join("c.txt"), "ignore").unwrap();

        let files = collect_files(temp.path(), None, false).unwrap();
        assert!(files.iter().any(|f| f.ends_with("a.py")));
        assert!(files.iter().any(|f| f.ends_with("b.rs")));
        assert!(!files.iter().any(|f| f.ends_with("c.txt")));
    }

    #[test]
    fn test_analyze_rust_detects_unsafe_without_safety_comment() {
        let source = r#"
pub fn raw_copy(ptr: *mut u8) {
    unsafe { *ptr = 7; }
}
"#;
        let findings = analyze_rust_file(Path::new("src/lib.rs"), source);
        assert!(findings.iter().any(|f| f.vuln_type == VulnType::UnsafeCode));
    }

    #[test]
    fn test_analyze_rust_detects_command_and_sql_patterns() {
        let source = r#"
use std::process::Command;

pub fn run(user: &str, name: &str) {
    let q = format!("SELECT * FROM users WHERE name = '{}'", name);
    let _ = Command::new("sh").arg(user).output();
}
"#;
        let findings = analyze_rust_file(Path::new("src/lib.rs"), source);
        assert!(findings
            .iter()
            .any(|f| f.vuln_type == VulnType::SqlInjection));
        assert!(findings
            .iter()
            .any(|f| f.vuln_type == VulnType::CommandInjection));
    }

    #[test]
    fn test_analyze_rust_detects_transmute_usage() {
        let source = r#"
use std::mem;

pub fn cast(x: u32) -> i32 {
    unsafe { mem::transmute(x) }
}
"#;
        let findings = analyze_rust_file(Path::new("src/lib.rs"), source);
        assert!(findings
            .iter()
            .any(|f| f.vuln_type == VulnType::MemorySafety && f.title.contains("transmute")));
    }

    #[test]
    fn test_analyze_rust_detects_raw_pointer_operation() {
        let source = r#"
pub unsafe fn read_ptr(p: *const u8) -> u8 {
    std::ptr::read(p)
}
"#;
        let findings = analyze_rust_file(Path::new("src/lib.rs"), source);
        assert!(findings
            .iter()
            .any(|f| f.vuln_type == VulnType::MemorySafety && f.title.contains("Raw Pointer")));
    }

    #[test]
    fn test_analyze_rust_detects_unwrap_in_non_test_code() {
        let source = r#"
pub fn parse(s: &str) -> i32 {
    s.parse::<i32>().unwrap()
}
"#;
        let findings = analyze_rust_file(Path::new("src/lib.rs"), source);
        assert!(findings
            .iter()
            .any(|f| f.vuln_type == VulnType::Panic && f.title.contains("unwrap")));
    }

    #[test]
    fn test_analyze_rust_detects_unchecked_bytes_patterns() {
        let source = r#"
pub fn from_raw(bytes: &[u8]) -> &str {
    unsafe { std::str::from_utf8_unchecked(bytes) }
}
"#;
        let findings = analyze_rust_file(Path::new("src/lib.rs"), source);
        assert!(findings
            .iter()
            .any(|f| f.vuln_type == VulnType::MemorySafety
                && f.title.contains("Unchecked Byte/String Conversion")));
    }
}
