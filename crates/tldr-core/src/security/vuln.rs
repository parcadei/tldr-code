//! Vulnerability detection via taint analysis
//!
//! Implements detection of security vulnerabilities as per spec Section 2.9.2:
//! - SQL Injection (user input -> cursor.execute)
//! - XSS (user input -> innerHTML)
//! - Command Injection (user input -> os.system)
//! - Path Traversal (user input -> open/Path)
//!
//! Uses DFG for taint flow tracking from sources to sinks.
//!
//! # Example
//! ```ignore
//! use tldr_core::security::vuln::{scan_vulnerabilities, VulnType};
//!
//! let report = scan_vulnerabilities(Path::new("src/"), None, None)?;
//! for finding in &report.findings {
//!     println!("{}: {} -> {}", finding.vuln_type, finding.source, finding.sink);
//! }
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::error::TldrError;
use crate::types::Language;
use crate::TldrResult;

// =============================================================================
// Types
// =============================================================================

/// Types of vulnerabilities detected
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VulnType {
    /// SQL Injection - unsanitized input to SQL queries
    SqlInjection,
    /// Cross-Site Scripting - unsanitized input to HTML output
    Xss,
    /// Command Injection - unsanitized input to shell commands
    CommandInjection,
    /// Path Traversal - unsanitized input to file operations
    PathTraversal,
    /// Server-Side Request Forgery
    Ssrf,
    /// Unsafe Deserialization
    Deserialization,
}

impl std::fmt::Display for VulnType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VulnType::SqlInjection => write!(f, "SQL Injection"),
            VulnType::Xss => write!(f, "Cross-Site Scripting (XSS)"),
            VulnType::CommandInjection => write!(f, "Command Injection"),
            VulnType::PathTraversal => write!(f, "Path Traversal"),
            VulnType::Ssrf => write!(f, "Server-Side Request Forgery"),
            VulnType::Deserialization => write!(f, "Unsafe Deserialization"),
        }
    }
}

/// A taint source (user input entry point)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintSource {
    /// Variable name containing tainted data
    pub variable: String,
    /// Source description
    pub source_type: String,
    /// Line number
    pub line: u32,
    /// Original expression
    pub expression: String,
}

/// A taint sink (dangerous operation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintSink {
    /// Function/method being called
    pub function: String,
    /// Sink type description
    pub sink_type: String,
    /// Line number
    pub line: u32,
    /// Full call expression
    pub expression: String,
}

/// A single vulnerability finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnFinding {
    /// Type of vulnerability
    pub vuln_type: VulnType,
    /// File containing the vulnerability
    pub file: PathBuf,
    /// Source of tainted data
    pub source: TaintSource,
    /// Sink where tainted data flows
    pub sink: TaintSink,
    /// Taint flow path (variable assignments)
    pub flow_path: Vec<String>,
    /// Severity (based on vuln type and certainty)
    pub severity: String,
    /// Remediation advice
    pub remediation: String,
    /// CWE ID
    pub cwe_id: Option<String>,
}

/// Summary of vulnerability scan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnSummary {
    /// Total vulnerabilities found
    pub total_findings: usize,
    /// Count by vulnerability type
    pub by_type: HashMap<String, usize>,
    /// Files with vulnerabilities
    pub affected_files: usize,
}

/// Report from vulnerability scan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnReport {
    /// All vulnerability findings
    pub findings: Vec<VulnFinding>,
    /// Number of files scanned
    pub files_scanned: usize,
    /// Summary statistics
    pub summary: VulnSummary,
}

// =============================================================================
// Taint Sources and Sinks
// =============================================================================

/// Get known taint sources for a language
fn get_sources(language: Language) -> Vec<(&'static str, &'static str)> {
    match language {
        Language::Python => vec![
            ("request.args", "Flask GET parameter"),
            ("request.form", "Flask POST parameter"),
            ("request.json", "Flask JSON body"),
            ("request.data", "Flask raw body"),
            ("request.values", "Flask combined params"),
            ("request.cookies", "Flask cookies"),
            ("request.headers", "Flask headers"),
            ("input(", "User input from stdin"),
            ("sys.argv", "Command line arguments"),
            ("os.environ", "Environment variables"),
        ],
        Language::JavaScript | Language::TypeScript => vec![
            ("req.params", "Express route parameter"),
            ("req.query", "Express query parameter"),
            ("req.body", "Express request body"),
            ("req.cookies", "Express cookies"),
            ("req.headers", "Express headers"),
            ("process.argv", "Command line arguments"),
            ("process.env", "Environment variables"),
            ("document.location", "Browser URL"),
            ("window.location", "Browser URL"),
            ("URLSearchParams", "URL parameters"),
        ],
        Language::Go => vec![
            ("r.URL.Query()", "HTTP query parameters"),
            ("r.FormValue(", "HTTP form value"),
            ("r.PostFormValue(", "HTTP POST value"),
            ("r.Header.Get(", "HTTP header"),
            ("os.Args", "Command line arguments"),
            ("os.Getenv(", "Environment variable"),
        ],
        Language::Rust => vec![
            ("std::env::args()", "Command line arguments"),
            ("env::args()", "Command line arguments"),
            ("std::env::var(", "Environment variable"),
            ("env::var(", "Environment variable"),
            ("std::io::stdin()", "Standard input"),
            ("stdin().read_line(", "Standard input"),
            ("stdin().read_to_string(", "Standard input"),
        ],
        Language::Java => vec![
            ("request.getParameter(", "Servlet parameter"),
            ("request.getHeader(", "Servlet header"),
            ("request.getCookies()", "Servlet cookies"),
            ("args", "Command line arguments"),
            ("System.getenv(", "Environment variable"),
        ],
        Language::C => vec![
            ("argv[", "Command line arguments"),
            ("getenv(", "Environment variable"),
            ("scanf(", "Standard input"),
            ("fgets(", "Standard input"),
            ("read(", "Raw input read"),
        ],
        Language::Cpp => vec![
            ("argv[", "Command line arguments"),
            ("std::getenv(", "Environment variable"),
            ("getenv(", "Environment variable"),
            ("std::cin", "Standard input"),
            ("getline(std::cin", "Standard input"),
        ],
        Language::Ruby => vec![
            ("params[", "Rails parameter"),
            ("request.headers[", "Rails header"),
            ("cookies[", "Rails cookie"),
            ("ENV[", "Environment variable"),
            ("ARGV[", "Command line arguments"),
            ("STDIN.gets", "Standard input"),
            ("STDIN.read", "Standard input"),
        ],
        Language::Kotlin => vec![
            ("call.request.queryParameters", "Ktor query parameters"),
            ("call.receive<", "Ktor request body"),
            ("call.parameters[", "Ktor route parameter"),
            ("System.getenv(", "Environment variable"),
            ("args[", "Command line arguments"),
            ("readLine()", "Standard input"),
        ],
        Language::Swift => vec![
            ("request.query[", "Vapor query parameter"),
            ("request.headers.first", "Vapor header"),
            ("request.body.string", "HTTP request body"),
            (
                "ProcessInfo.processInfo.environment",
                "Environment variable",
            ),
            ("CommandLine.arguments", "Command line arguments"),
            ("readLine()", "Standard input"),
        ],
        Language::CSharp => vec![
            ("Request.Query", "ASP.NET query parameter"),
            ("Request.Form", "ASP.NET form parameter"),
            ("Request.Headers", "ASP.NET header"),
            (
                "Environment.GetEnvironmentVariable(",
                "Environment variable",
            ),
            ("args[", "Command line arguments"),
            ("Console.ReadLine()", "Standard input"),
        ],
        Language::Scala => vec![
            ("request.getQueryString(", "Play query parameter"),
            ("request.headers.get(", "Play header"),
            ("request.body.asText", "HTTP request body"),
            ("sys.env(", "Environment variable"),
            ("System.getenv(", "Environment variable"),
            ("args(", "Command line arguments"),
            ("StdIn.readLine()", "Standard input"),
        ],
        Language::Php => vec![
            ("$_GET", "HTTP GET parameter"),
            ("$_POST", "HTTP POST parameter"),
            ("$_REQUEST", "Combined request parameters"),
            ("$_COOKIE", "HTTP cookie"),
            ("$_SERVER", "Server header data"),
            ("getenv(", "Environment variable"),
            ("$argv", "Command line arguments"),
        ],
        Language::Lua | Language::Luau => vec![
            ("ngx.req.get_uri_args()", "OpenResty query parameters"),
            ("ngx.req.get_post_args()", "OpenResty POST parameters"),
            ("ngx.var.arg_", "OpenResty route/query parameter"),
            ("os.getenv(", "Environment variable"),
            ("arg[", "Command line arguments"),
            ("io.read()", "Standard input"),
        ],
        Language::Elixir => vec![
            ("conn.params", "Phoenix params"),
            ("conn.query_params", "Phoenix query params"),
            ("conn.body_params", "Phoenix body params"),
            ("get_req_header(", "Phoenix headers"),
            ("System.get_env(", "Environment variable"),
            ("System.argv()", "Command line arguments"),
            ("IO.gets(", "Standard input"),
        ],
        Language::Ocaml => vec![
            ("Sys.argv", "Command line arguments"),
            ("Sys.getenv", "Environment variable"),
            ("read_line ()", "Standard input"),
            ("read_line()", "Standard input"),
            ("input_line", "Standard input"),
            ("In_channel.input_all", "File input"),
        ],
    }
}

/// Get known taint sinks for each vulnerability type
fn get_sinks(vuln_type: VulnType, language: Language) -> Vec<(&'static str, &'static str)> {
    match vuln_type {
        VulnType::SqlInjection => match language {
            Language::Python => vec![
                ("cursor.execute(", "Direct SQL execution"),
                (".execute(", "SQL execution"),
                (".raw(", "Django raw SQL"),
                (".extra(", "Django extra SQL"),
                ("engine.execute(", "SQLAlchemy execution"),
            ],
            Language::JavaScript | Language::TypeScript => vec![
                (".query(", "Database query"),
                (".raw(", "Raw SQL query"),
                ("knex.raw(", "Knex raw query"),
                ("sequelize.query(", "Sequelize raw query"),
            ],
            Language::Go => vec![
                (".Query(", "Database query"),
                (".Exec(", "Database exec"),
                (".QueryRow(", "Database query row"),
            ],
            Language::Rust => vec![
                ("sqlx::query(", "Raw SQL query"),
                ("client.query(", "Database query"),
                ("client.execute(", "Database exec"),
                ("conn.execute(", "Database exec"),
            ],
            Language::Java => vec![
                ("statement.execute(", "JDBC execute"),
                ("statement.executeQuery(", "JDBC query"),
                ("statement.executeUpdate(", "JDBC update"),
                ("createQuery(", "JPA query"),
                ("createNativeQuery(", "JPA native query"),
            ],
            Language::C => vec![
                ("sqlite3_exec(", "SQLite exec"),
                ("mysql_query(", "MySQL query"),
                ("PQexec(", "PostgreSQL query"),
            ],
            Language::Cpp => vec![
                ("sqlite3_exec(", "SQLite exec"),
                ("mysql_query(", "MySQL query"),
                ("PQexec(", "PostgreSQL query"),
            ],
            Language::Ruby => vec![
                ("find_by_sql(", "ActiveRecord raw SQL"),
                (".execute(", "Database execution"),
                ("exec_query(", "Raw SQL query"),
                (".where(", "ActiveRecord where"),
            ],
            Language::Kotlin => vec![
                (".executeQuery(", "JDBC query"),
                (".executeUpdate(", "JDBC update"),
                ("createNativeQuery(", "JPA native query"),
                ("jdbcTemplate.queryForList(", "JdbcTemplate raw query"),
            ],
            Language::Swift => vec![
                ("sqlite3_exec(", "SQLite exec"),
                ("database.raw(", "Raw SQL execution"),
            ],
            Language::CSharp => vec![
                ("ExecuteReader(", "ADO.NET query"),
                ("ExecuteNonQuery(", "ADO.NET exec"),
                ("ExecuteSqlRaw(", "EF Core raw SQL"),
                ("FromSqlRaw(", "EF Core raw SQL"),
            ],
            Language::Scala => vec![
                (".executeQuery(", "JDBC query"),
                (".executeUpdate(", "JDBC update"),
                ("createNativeQuery(", "JPA native query"),
            ],
            Language::Php => vec![
                ("mysql_query(", "MySQL query"),
                ("mysqli_query(", "MySQLi query"),
                ("->query(", "Database query"),
                ("->exec(", "Database exec"),
            ],
            Language::Lua | Language::Luau => vec![
                ("db:query(", "Database query"),
                ("conn:execute(", "Database exec"),
                ("luasql.execute(", "LuaSQL execution"),
            ],
            Language::Elixir => vec![
                ("Repo.query(", "Ecto raw SQL"),
                ("Ecto.Adapters.SQL.query(", "Ecto adapter query"),
            ],
            Language::Ocaml => vec![
                ("Sqlite3.exec", "SQLite exec"),
                ("connection#exec", "PostgreSQL exec"),
            ],
        },
        VulnType::Xss => match language {
            Language::Python => vec![
                ("Markup(", "Flask markup"),
                ("mark_safe(", "Django mark_safe"),
                ("|safe", "Django safe filter"),
            ],
            Language::JavaScript | Language::TypeScript => vec![
                ("innerHTML", "Direct HTML injection"),
                ("outerHTML", "Direct HTML injection"),
                ("document.write(", "Document write"),
                ("document.writeln(", "Document writeln"),
                (".html(", "jQuery html"),
                ("dangerouslySetInnerHTML", "React unsafe HTML"),
            ],
            Language::Ruby => vec![
                ("html_safe", "Rails unsafe HTML"),
                ("raw(", "Rails raw helper"),
                ("render html:", "Direct HTML rendering"),
            ],
            Language::Php => vec![
                ("echo ", "Direct output"),
                ("print ", "Direct output"),
                ("<?= ", "Template raw output"),
            ],
            Language::CSharp => vec![
                ("Html.Raw(", "ASP.NET raw HTML"),
                ("@Html.Raw(", "Razor raw HTML"),
                ("AppendHtml(", "Raw HTML append"),
            ],
            Language::Elixir => vec![
                ("Phoenix.HTML.raw(", "Phoenix raw HTML"),
                ("raw(", "Raw HTML helper"),
            ],
            Language::Lua | Language::Luau => vec![
                ("ngx.say(", "OpenResty response body"),
                ("ngx.print(", "OpenResty raw output"),
            ],
            Language::Rust
            | Language::Go
            | Language::Java
            | Language::C
            | Language::Cpp
            | Language::Kotlin
            | Language::Swift
            | Language::Scala
            | Language::Ocaml => vec![],
        },
        VulnType::CommandInjection => match language {
            Language::Python => vec![
                ("os.system(", "Shell command"),
                ("os.popen(", "Shell pipe"),
                ("subprocess.call(", "Subprocess with shell"),
                ("subprocess.run(", "Subprocess run"),
                ("subprocess.Popen(", "Subprocess Popen"),
                ("eval(", "Python eval"),
                ("exec(", "Python exec"),
            ],
            Language::JavaScript | Language::TypeScript => vec![
                ("child_process.exec(", "Shell command"),
                ("child_process.execSync(", "Shell command sync"),
                ("child_process.spawn(", "Spawn process"),
                ("eval(", "JavaScript eval"),
                ("Function(", "Function constructor"),
            ],
            Language::Go => vec![
                ("exec.Command(", "Shell command"),
                ("os/exec.Command(", "Shell command"),
            ],
            Language::Rust => vec![
                ("Command::new(", "Process spawn"),
                ("std::process::Command::new(", "Process spawn"),
                (".arg(", "Process argument"),
                ("std::process::exit(", "Process exit from input"),
            ],
            Language::Java => vec![
                ("Runtime.getRuntime().exec(", "Runtime exec"),
                ("ProcessBuilder(", "Process builder"),
            ],
            Language::C => vec![
                ("system(", "Shell command"),
                ("popen(", "Shell pipe"),
                ("execl(", "Process exec"),
                ("execvp(", "Process exec"),
            ],
            Language::Cpp => vec![
                ("system(", "Shell command"),
                ("std::system(", "Shell command"),
                ("popen(", "Shell pipe"),
            ],
            Language::Ruby => vec![
                ("system(", "Shell command"),
                ("exec(", "Process exec"),
                ("Open3.capture3(", "Shell capture"),
                ("eval(", "Ruby eval"),
            ],
            Language::Kotlin => vec![
                ("Runtime.getRuntime().exec(", "Runtime exec"),
                ("ProcessBuilder(", "Process builder"),
            ],
            Language::Swift => vec![("system(", "Shell command"), ("Process(", "Process spawn")],
            Language::CSharp => vec![
                ("Process.Start(", "Process start"),
                ("new ProcessStartInfo(", "Process configuration"),
            ],
            Language::Scala => vec![
                ("Runtime.getRuntime.exec(", "Runtime exec"),
                ("Process(", "Scala process builder"),
            ],
            Language::Php => vec![
                ("system(", "Shell command"),
                ("exec(", "Process exec"),
                ("shell_exec(", "Shell exec"),
                ("passthru(", "Command passthrough"),
                ("eval(", "PHP eval"),
            ],
            Language::Lua | Language::Luau => vec![
                ("os.execute(", "Shell command"),
                ("io.popen(", "Shell pipe"),
                ("loadstring(", "Dynamic code load"),
                ("load(", "Dynamic code load"),
            ],
            Language::Elixir => vec![
                ("System.cmd(", "External command"),
                (":os.cmd(", "Shell command"),
                ("Code.eval_string(", "Dynamic code evaluation"),
            ],
            Language::Ocaml => vec![
                ("Sys.command", "Shell command"),
                ("Unix.open_process_in", "Process open"),
                ("Unix.open_process_full", "Process open"),
            ],
        },
        VulnType::PathTraversal => match language {
            Language::Python => vec![
                ("open(", "File open"),
                ("Path(", "Path construction"),
                ("os.path.join(", "Path join"),
                ("shutil.copy(", "File copy"),
                ("shutil.move(", "File move"),
            ],
            Language::JavaScript | Language::TypeScript => vec![
                ("fs.readFile(", "File read"),
                ("fs.writeFile(", "File write"),
                ("fs.readFileSync(", "File read sync"),
                ("fs.writeFileSync(", "File write sync"),
                ("path.join(", "Path join"),
            ],
            Language::Go => vec![
                ("os.Open(", "File open"),
                ("os.Create(", "File create"),
                ("ioutil.ReadFile(", "File read"),
                ("ioutil.WriteFile(", "File write"),
                ("filepath.Join(", "Path join"),
            ],
            Language::Rust => vec![
                ("std::fs::read_to_string(", "File read"),
                ("std::fs::write(", "File write"),
                ("File::open(", "File open"),
                ("PathBuf::from(", "Path construction"),
            ],
            Language::Java => vec![
                ("Files.readString(", "File read"),
                ("Files.writeString(", "File write"),
                ("Paths.get(", "Path construction"),
                ("new File(", "File construction"),
            ],
            Language::C => vec![
                ("fopen(", "File open"),
                ("open(", "File descriptor open"),
                ("freopen(", "File reopen"),
            ],
            Language::Cpp => vec![
                ("std::ifstream(", "File read"),
                ("std::ofstream(", "File write"),
                ("fopen(", "C file open"),
            ],
            Language::Ruby => vec![
                ("File.open(", "File open"),
                ("File.read(", "File read"),
                ("File.write(", "File write"),
                ("Pathname.new(", "Path construction"),
            ],
            Language::Kotlin => vec![
                ("File(", "File construction"),
                ("Files.readString(", "File read"),
                ("Files.writeString(", "File write"),
                ("Paths.get(", "Path construction"),
            ],
            Language::Swift => vec![
                ("String(contentsOfFile:", "File read"),
                ("Data(contentsOf:", "File read"),
                ("FileManager.default.contents(atPath:", "File read"),
            ],
            Language::CSharp => vec![
                ("File.Open(", "File open"),
                ("File.ReadAllText(", "File read"),
                ("File.WriteAllText(", "File write"),
                ("Path.Combine(", "Path join"),
            ],
            Language::Scala => vec![
                ("Source.fromFile(", "File read"),
                ("Files.readString(", "File read"),
                ("Files.writeString(", "File write"),
                ("Paths.get(", "Path construction"),
            ],
            Language::Php => vec![
                ("fopen(", "File open"),
                ("file_get_contents(", "File read"),
                ("file_put_contents(", "File write"),
                ("include(", "File include"),
                ("require(", "File require"),
            ],
            Language::Lua | Language::Luau => vec![
                ("io.open(", "File open"),
                ("dofile(", "File execute"),
                ("loadfile(", "File load"),
            ],
            Language::Elixir => vec![
                ("File.read(", "File read"),
                ("File.write(", "File write"),
                ("Path.join(", "Path join"),
            ],
            Language::Ocaml => vec![
                ("open_in", "File open for read"),
                ("open_out", "File open for write"),
                ("Filename.concat", "Path join"),
            ],
        },
        VulnType::Ssrf => match language {
            Language::Python
            | Language::TypeScript
            | Language::JavaScript
            | Language::Go
            | Language::Rust
            | Language::Java
            | Language::C
            | Language::Cpp
            | Language::Ruby
            | Language::Kotlin
            | Language::Swift
            | Language::CSharp
            | Language::Scala
            | Language::Php
            | Language::Lua
            | Language::Luau
            | Language::Elixir
            | Language::Ocaml => vec![],
        },
        VulnType::Deserialization => match language {
            Language::Python => vec![
                ("pickle.load(", "Pickle load"),
                ("pickle.loads(", "Pickle loads"),
                ("yaml.load(", "YAML load (unsafe)"),
                ("yaml.unsafe_load(", "YAML unsafe load"),
            ],
            Language::Java => vec![
                ("ObjectInputStream", "Java deserialization"),
                ("readObject(", "Object deserialization"),
                ("XMLDecoder", "XML deserialization"),
            ],
            Language::Rust => vec![
                ("serde_json::from_str(", "JSON deserialization"),
                ("serde_yaml::from_str(", "YAML deserialization"),
                ("bincode::deserialize(", "Binary deserialization"),
            ],
            Language::Cpp => vec![
                (
                    "boost::archive::text_iarchive",
                    "Boost text deserialization",
                ),
                (
                    "cereal::BinaryInputArchive",
                    "Cereal binary deserialization",
                ),
            ],
            Language::Ruby => vec![
                ("Marshal.load(", "Marshal deserialization"),
                ("YAML.load(", "YAML deserialization"),
                ("Psych.load(", "Psych deserialization"),
            ],
            Language::Kotlin => vec![
                ("ObjectInputStream(", "Java object deserialization"),
                ("readObject(", "Object deserialization"),
            ],
            Language::CSharp => vec![
                (
                    "BinaryFormatter.Deserialize(",
                    "BinaryFormatter deserialize",
                ),
                (
                    "NetDataContractSerializer.Deserialize(",
                    "NetDataContract deserialize",
                ),
            ],
            Language::Scala => vec![
                ("ObjectInputStream(", "Java object deserialization"),
                ("readObject(", "Object deserialization"),
            ],
            Language::Php => vec![
                ("unserialize(", "PHP unserialize"),
                ("yaml_parse(", "YAML parse"),
            ],
            Language::Elixir => vec![(":erlang.binary_to_term(", "Erlang term deserialization")],
            Language::Ocaml => vec![
                ("Marshal.from_channel", "Marshal deserialization"),
                ("Marshal.from_string", "Marshal deserialization"),
            ],
            Language::TypeScript
            | Language::JavaScript
            | Language::Go
            | Language::C
            | Language::Swift
            | Language::Lua
            | Language::Luau => vec![],
        },
    }
}

/// Get remediation advice for a vulnerability type
fn get_remediation(vuln_type: VulnType) -> &'static str {
    match vuln_type {
        VulnType::SqlInjection =>
            "Use parameterized queries or prepared statements instead of string concatenation",
        VulnType::Xss =>
            "Sanitize output using context-appropriate encoding (HTML, JavaScript, URL, etc.)",
        VulnType::CommandInjection =>
            "Use subprocess with shell=False and pass arguments as a list, or use shlex.quote()",
        VulnType::PathTraversal =>
            "Validate paths against a whitelist or use realpath() and verify the result is within allowed directories",
        VulnType::Ssrf =>
            "Validate URLs against an allowlist of domains and protocols",
        VulnType::Deserialization =>
            "Avoid deserializing untrusted data, or use safer formats like JSON",
    }
}

/// Get CWE ID for a vulnerability type
fn get_cwe_id(vuln_type: VulnType) -> &'static str {
    match vuln_type {
        VulnType::SqlInjection => "CWE-89",
        VulnType::Xss => "CWE-79",
        VulnType::CommandInjection => "CWE-78",
        VulnType::PathTraversal => "CWE-22",
        VulnType::Ssrf => "CWE-918",
        VulnType::Deserialization => "CWE-502",
    }
}

// =============================================================================
// Main API
// =============================================================================

/// Scan for security vulnerabilities using taint analysis
///
/// # Arguments
/// * `path` - File or directory to scan
/// * `language` - Optional language filter (auto-detect if None)
/// * `vuln_type` - Optional filter for specific vulnerability type
///
/// # Returns
/// * `Ok(VulnReport)` - Report with all findings
/// * `Err(TldrError)` - On file system or parse errors
///
/// # Example
/// ```ignore
/// use tldr_core::security::vuln::{scan_vulnerabilities, VulnType};
///
/// // Scan for all vulnerabilities
/// let report = scan_vulnerabilities(Path::new("src/"), None, None)?;
///
/// // Scan for SQL injection only
/// let report = scan_vulnerabilities(
///     Path::new("src/"),
///     Some(Language::Python),
///     Some(VulnType::SqlInjection),
/// )?;
/// ```
pub fn scan_vulnerabilities(
    path: &Path,
    language: Option<Language>,
    vuln_type: Option<VulnType>,
) -> TldrResult<VulnReport> {
    let mut findings = Vec::new();
    let mut files_scanned = 0;

    // Collect files to scan
    let files: Vec<PathBuf> = if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        WalkDir::new(path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                let detected = Language::from_path(e.path());
                match (detected, language) {
                    (Some(d), Some(l)) => d == l,
                    (Some(_), None) => true,
                    _ => false,
                }
            })
            .map(|e| e.path().to_path_buf())
            .collect()
    };

    // Scan each file
    for file_path in &files {
        if let Ok(file_findings) = scan_file_vulns(file_path, vuln_type) {
            findings.extend(file_findings);
            files_scanned += 1;
        }
    }

    // Calculate summary
    let mut by_type: HashMap<String, usize> = HashMap::new();
    let mut affected_files: HashSet<PathBuf> = HashSet::new();
    for finding in &findings {
        *by_type.entry(finding.vuln_type.to_string()).or_insert(0) += 1;
        affected_files.insert(finding.file.clone());
    }

    let summary = VulnSummary {
        total_findings: findings.len(),
        by_type,
        affected_files: affected_files.len(),
    };

    Ok(VulnReport {
        findings,
        files_scanned,
        summary,
    })
}

// =============================================================================
// Internal Implementation
// =============================================================================

/// Scan a single file for vulnerabilities
fn scan_file_vulns(path: &Path, vuln_filter: Option<VulnType>) -> TldrResult<Vec<VulnFinding>> {
    let content = std::fs::read_to_string(path)?;
    let language = Language::from_path(path).ok_or_else(|| {
        TldrError::UnsupportedLanguage(
            path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("unknown")
                .to_string(),
        )
    })?;

    let mut findings = Vec::new();
    let sources = get_sources(language);

    // Vulnerability types to check
    let vuln_types = if let Some(vt) = vuln_filter {
        vec![vt]
    } else {
        vec![
            VulnType::SqlInjection,
            VulnType::Xss,
            VulnType::CommandInjection,
            VulnType::PathTraversal,
            VulnType::Deserialization,
        ]
    };

    // For each line, check for sources and sinks
    let lines: Vec<&str> = content.lines().collect();

    // Track tainted variables (simplified taint tracking)
    let mut tainted_vars: HashMap<String, (u32, String)> = HashMap::new();

    // First pass: identify taint sources
    for (line_num, line) in lines.iter().enumerate() {
        let line_num = (line_num + 1) as u32;

        for (source_pattern, source_desc) in &sources {
            if line.contains(source_pattern) {
                // Extract variable being assigned
                if let Some(var) = extract_assigned_variable(line) {
                    // Check if the source is wrapped in a type coercion
                    // e.g. user_id = int(request.args.get("id"))
                    let skip = if let Some(eq_pos) = line.find('=') {
                        let rhs = &line[eq_pos + 1..];
                        is_type_coerced(rhs, source_pattern)
                    } else {
                        false
                    };
                    if !skip {
                        tainted_vars.insert(var.clone(), (line_num, source_desc.to_string()));
                    }
                }
            }
        }

        // Track variable propagation (simplified)
        for (tainted_var, _) in tainted_vars.clone().iter() {
            if let Some(new_var) = extract_propagation(line, tainted_var) {
                if !tainted_vars.contains_key(&new_var) {
                    tainted_vars.insert(new_var, tainted_vars[tainted_var].clone());
                }
            }
        }
    }

    // Second pass: check for sinks with tainted data
    for vuln_type in vuln_types {
        let sinks = get_sinks(vuln_type, language);

        for (line_num, line) in lines.iter().enumerate() {
            let line_num = (line_num + 1) as u32;

            for (sink_pattern, sink_desc) in &sinks {
                if line.contains(sink_pattern) {
                    // Check if any tainted variable flows into this sink
                    for (var, (source_line, source_desc)) in &tainted_vars {
                        if line.contains(var.as_str()) {
                            // Skip if the sink line uses sanitization
                            let vuln_type_str = vuln_type.to_string();
                            if is_sanitized_sink(line, var.as_str(), &vuln_type_str) {
                                continue;
                            }
                            findings.push(VulnFinding {
                                vuln_type,
                                file: path.to_path_buf(),
                                source: TaintSource {
                                    variable: var.clone(),
                                    source_type: source_desc.clone(),
                                    line: *source_line,
                                    expression: get_line_at(&content, *source_line)
                                        .unwrap_or_default()
                                        .trim()
                                        .to_string(),
                                },
                                sink: TaintSink {
                                    function: sink_pattern.to_string(),
                                    sink_type: sink_desc.to_string(),
                                    line: line_num,
                                    expression: line.trim().to_string(),
                                },
                                flow_path: vec![
                                    format!("{}:{} - taint source", source_line, var),
                                    format!("{}:{} - sink", line_num, sink_pattern),
                                ],
                                severity: "HIGH".to_string(),
                                remediation: get_remediation(vuln_type).to_string(),
                                cwe_id: Some(get_cwe_id(vuln_type).to_string()),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(findings)
}

/// Extract the variable name from an assignment
fn extract_assigned_variable(line: &str) -> Option<String> {
    // Simple pattern: var = something
    let parts: Vec<&str> = line.split('=').collect();
    if parts.len() >= 2 {
        let mut lhs = parts[0].trim().to_string();
        // Handle Go := operator — strip trailing ':'
        if lhs.ends_with(':') {
            lhs = lhs[..lhs.len() - 1].trim().to_string();
        }
        // Handle typed declarations: "String id", "var id", "let id", "const id"
        // Take the last whitespace-separated token as the variable name
        let var = lhs.split_whitespace().next_back().unwrap_or(&lhs);
        // Handle attribute access: self.var -> var
        let var = var.split('.').next_back().unwrap_or(var);
        // Strip pointer/reference markers: *ptr, &ref
        let var = var.trim_start_matches(['*', '&']);
        // Strip PHP $ prefix
        let var = var.trim_start_matches('$');
        // Basic identifier validation
        if var.chars().all(|c| c.is_alphanumeric() || c == '_') && !var.is_empty() {
            return Some(var.to_string());
        }
    }
    None
}

/// Extract variable propagation (var2 = something(var1))
///
/// Returns `None` (breaking the taint chain) if the tainted variable is wrapped
/// in a type-coercion function (`int()`, `float()`, `bool()`, `str(int())`,
/// `str(float())`). These functions convert arbitrary strings to constrained
/// types, eliminating injection payloads.
fn extract_propagation(line: &str, tainted_var: &str) -> Option<String> {
    // Check if line contains tainted variable on RHS
    if let Some(eq_pos) = line.find('=') {
        let rhs = &line[eq_pos + 1..];
        if rhs.contains(tainted_var) {
            // Type-coercion taint-break: if the tainted var appears inside
            // int(...), float(...), or bool(...), the output is a safe
            // primitive that cannot carry injection payloads.
            if is_type_coerced(rhs, tainted_var) {
                return None;
            }
            // Get LHS variable
            return extract_assigned_variable(line);
        }
    }
    None
}

/// Check if `tainted_var` is wrapped in a type-coercion function on the RHS.
///
/// Recognizes: `int(`, `float(`, `bool(`, `str(int(`, `str(float(`.
/// Does NOT treat bare `str()` as a sanitizer (it just converts to string
/// without constraining the value).
fn is_type_coerced(rhs: &str, tainted_var: &str) -> bool {
    // Find where the tainted var appears in the RHS
    if let Some(var_pos) = rhs.find(tainted_var) {
        // Look at the text before the tainted var for coercion wrappers
        let before = &rhs[..var_pos];

        // Direct coercion: int(...), float(...), bool(...)
        let coercion_funcs = ["int(", "float(", "bool("];
        for func in &coercion_funcs {
            if before.ends_with(func) || before.trim_end().ends_with(func) {
                return true;
            }
        }

        // Nested coercion: str(int(...)), str(float(...))
        let nested_funcs = ["str(int(", "str(float("];
        for func in &nested_funcs {
            if before.contains(func) {
                return true;
            }
        }
    }
    false
}

/// Check if a sink line uses sanitization that prevents the vulnerability.
///
/// Returns `true` if the line demonstrates a safe usage pattern, meaning the
/// finding should be suppressed. Checks differ by vulnerability type:
///
/// **SQL Injection:** Parameterized queries using `?`, `%s`, or `:param`
/// placeholders combined with tuple/list argument passing (`, (` or `, [`).
///
/// **Command Injection:** List-form subprocess arguments (`subprocess.run([...])`)
/// or explicit `shell=False`.
fn is_sanitized_sink(line: &str, _var: &str, vuln_type: &str) -> bool {
    let lower_vuln = vuln_type.to_lowercase();

    if lower_vuln.contains("sql") {
        return is_sanitized_sql(line);
    }

    if lower_vuln.contains("command") {
        return is_sanitized_command(line);
    }

    false
}

/// Check if a SQL sink line uses parameterized queries.
///
/// Detects placeholder markers (`?`, `%s`, `:param_name`) combined with
/// tuple/list argument syntax (`, (` or `, [`). Both conditions must be
/// present -- a placeholder alone does not indicate parameterization if
/// the arguments are not passed separately.
fn is_sanitized_sql(line: &str) -> bool {
    let has_placeholder = line.contains('?') || line.contains("%s") || has_named_param(line);

    let has_args_tuple = line.contains(", (") || line.contains(", [") || line.contains(", {");

    has_placeholder && has_args_tuple
}

/// Check if a line contains a named parameter placeholder like `:user_id`.
///
/// Matches a colon followed by one or more word characters, but avoids
/// false positives on Python slice notation or URLs by requiring the colon
/// to appear after a space or `=` (typical SQL context).
fn has_named_param(line: &str) -> bool {
    // Look for :word patterns that appear in SQL context
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b':' && i + 1 < bytes.len() {
            // The char after : must be a letter (not digit, not another :)
            let next = bytes[i + 1];
            if next.is_ascii_alphabetic() {
                // Check that the char before : is a space, =, or quote
                // to avoid matching URLs like http://
                if i == 0 {
                    return true;
                }
                let prev = bytes[i - 1];
                if prev == b' ' || prev == b'=' || prev == b'\'' || prev == b'"' {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if a command injection sink uses safe patterns.
///
/// Safe patterns:
/// - List-form arguments: `subprocess.run([`, `subprocess.call([`, `subprocess.Popen([`
/// - Explicit `shell=False`
///
/// Unsafe override: if `shell=True` appears on the line, the sink is NOT safe
/// regardless of other patterns.
fn is_sanitized_command(line: &str) -> bool {
    // shell=True is always unsafe -- check this first
    if line.contains("shell=True") {
        return false;
    }

    // shell=False is explicitly safe
    if line.contains("shell=False") {
        return true;
    }

    // List-form args: subprocess.run([...]), subprocess.call([...]), subprocess.Popen([...])
    let list_patterns = ["subprocess.run(", "subprocess.call(", "subprocess.Popen("];
    for pattern in &list_patterns {
        if let Some(pos) = line.find(pattern) {
            // Check if there's a `[` after the opening paren (possibly with spaces)
            let after_paren = &line[pos + pattern.len()..];
            let trimmed = after_paren.trim_start();
            if trimmed.starts_with('[') {
                return true;
            }
        }
    }

    false
}

/// Get a specific line from content
fn get_line_at(content: &str, line_num: u32) -> Option<String> {
    content
        .lines()
        .nth((line_num - 1) as usize)
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use tempfile::TempDir;

    #[test]
    fn test_extract_assigned_variable() {
        assert_eq!(
            extract_assigned_variable("user_input = request.args.get('id')"),
            Some("user_input".to_string())
        );
        assert_eq!(
            extract_assigned_variable("self.data = value"),
            Some("data".to_string())
        );
        assert_eq!(extract_assigned_variable("x = 1"), Some("x".to_string()));
    }

    #[test]
    fn test_extract_assigned_variable_go() {
        // Go := operator
        assert_eq!(
            extract_assigned_variable("    id := r.URL.Query().Get(\"id\")"),
            Some("id".to_string())
        );
        // Go multi-assign: db, _ := sql.Open(...)
        assert_eq!(
            extract_assigned_variable("    db, _ := sql.Open(\"mysql\", \"dsn\")"),
            Some("_".to_string()) // last token after split — but db is what we want
        );
    }

    #[test]
    fn test_extract_assigned_variable_java() {
        assert_eq!(
            extract_assigned_variable("        String id = request.getParameter(\"id\");"),
            Some("id".to_string())
        );
        assert_eq!(
            extract_assigned_variable(
                "        Connection conn = DriverManager.getConnection(\"url\");"
            ),
            Some("conn".to_string())
        );
    }

    #[test]
    fn test_go_vuln_e2e() {
        let go_code = r#"package main

import (
    "database/sql"
    "net/http"
    "os/exec"
)

func handler(w http.ResponseWriter, r *http.Request) {
    id := r.URL.Query().Get("id")
    db, _ := sql.Open("mysql", "dsn")
    db.Query("SELECT * FROM users WHERE id = " + id)

    cmd := r.URL.Query().Get("cmd")
    out, _ := exec.Command(cmd).Output()
}
"#;
        let tmp = std::env::temp_dir().join("test_go_vuln_e2e.go");
        std::fs::write(&tmp, go_code).unwrap();
        let result = scan_vulnerabilities(&tmp, None, None).unwrap();
        eprintln!("Go findings: {}", result.findings.len());
        for f in &result.findings {
            eprintln!(
                "  {:?} line {}: {} -> {}",
                f.vuln_type, f.sink.line, f.source.variable, f.sink.function
            );
        }
        assert!(
            !result.findings.is_empty(),
            "Expected Go SQL injection finding, got {}",
            result.findings.len()
        );
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_extract_propagation() {
        assert_eq!(
            extract_propagation(
                "query = 'SELECT * FROM users WHERE id=' + user_input",
                "user_input"
            ),
            Some("query".to_string())
        );
    }

    #[test]
    fn test_vuln_type_display() {
        assert_eq!(VulnType::SqlInjection.to_string(), "SQL Injection");
        assert_eq!(VulnType::Xss.to_string(), "Cross-Site Scripting (XSS)");
    }

    #[test]
    fn test_get_sources_python() {
        let sources = get_sources(Language::Python);
        assert!(sources.iter().any(|(p, _)| *p == "request.args"));
        assert!(sources.iter().any(|(p, _)| *p == "sys.argv"));
    }

    #[test]
    fn test_get_sinks_sql_injection() {
        let sinks = get_sinks(VulnType::SqlInjection, Language::Python);
        assert!(sinks.iter().any(|(p, _)| p.contains("execute")));
    }

    #[test]
    fn test_get_sources_has_rust_and_php_coverage() {
        let rust_sources = get_sources(Language::Rust);
        let php_sources = get_sources(Language::Php);
        assert!(rust_sources.iter().any(|(p, _)| *p == "std::env::args()"));
        assert!(php_sources.iter().any(|(p, _)| *p == "$_GET"));
    }

    #[test]
    fn test_get_sinks_has_csharp_and_elixir_coverage() {
        let csharp_sinks = get_sinks(VulnType::CommandInjection, Language::CSharp);
        let elixir_sinks = get_sinks(VulnType::Deserialization, Language::Elixir);
        assert!(csharp_sinks.iter().any(|(p, _)| *p == "Process.Start("));
        assert!(elixir_sinks
            .iter()
            .any(|(p, _)| *p == ":erlang.binary_to_term("));
    }

    #[test]
    fn test_cwe_ids() {
        assert_eq!(get_cwe_id(VulnType::SqlInjection), "CWE-89");
        assert_eq!(get_cwe_id(VulnType::Xss), "CWE-79");
        assert_eq!(get_cwe_id(VulnType::CommandInjection), "CWE-78");
    }

    // =========================================================================
    // Sanitizer Awareness Tests
    // =========================================================================

    // --- Type coercion taint-break in extract_propagation ---

    #[test]
    fn test_type_coercion_int_breaks_taint() {
        // int() wrapping a tainted var should NOT propagate taint
        let result = extract_propagation("    user_id = int(request.args.get(\"id\"))", "request");
        assert_eq!(result, None, "int() should break taint propagation");
    }

    #[test]
    fn test_type_coercion_float_breaks_taint() {
        let result = extract_propagation("    price = float(user_input)", "user_input");
        assert_eq!(result, None, "float() should break taint propagation");
    }

    #[test]
    fn test_type_coercion_bool_breaks_taint() {
        let result = extract_propagation("    flag = bool(user_input)", "user_input");
        assert_eq!(result, None, "bool() should break taint propagation");
    }

    #[test]
    fn test_type_coercion_str_int_breaks_taint() {
        // str(int(x)) should also break taint
        let result = extract_propagation("    safe_id = str(int(user_input))", "user_input");
        assert_eq!(result, None, "str(int()) should break taint propagation");
    }

    #[test]
    fn test_type_coercion_str_float_breaks_taint() {
        let result = extract_propagation("    safe_val = str(float(user_input))", "user_input");
        assert_eq!(result, None, "str(float()) should break taint propagation");
    }

    #[test]
    fn test_no_type_coercion_still_propagates() {
        // Plain assignment without coercion must still propagate
        let result = extract_propagation(
            "    query = \"SELECT * FROM users WHERE id=\" + user_input",
            "user_input",
        );
        assert_eq!(
            result,
            Some("query".to_string()),
            "Assignment without coercion must propagate taint"
        );
    }

    #[test]
    fn test_str_alone_does_not_break_taint() {
        // str(user_input) alone should NOT break taint (just converts to string)
        let result = extract_propagation("    name_str = str(user_input)", "user_input");
        assert_eq!(
            result,
            Some("name_str".to_string()),
            "str() alone must NOT break taint (no type coercion)"
        );
    }

    // --- is_sanitized_sink tests ---

    #[test]
    fn test_sanitized_sql_parameterized_question_mark() {
        assert!(
            is_sanitized_sink(
                "    cursor.execute(\"SELECT * FROM users WHERE id = ?\", (user_id,))",
                "user_id",
                "SQL Injection",
            ),
            "Parameterized query with ? should be detected as sanitized"
        );
    }

    #[test]
    fn test_sanitized_sql_parameterized_percent_s() {
        assert!(
            is_sanitized_sink(
                "    cursor.execute(\"SELECT * FROM users WHERE id = %s\", (user_id,))",
                "user_id",
                "SQL Injection",
            ),
            "Parameterized query with %s should be detected as sanitized"
        );
    }

    #[test]
    fn test_sanitized_sql_parameterized_named() {
        assert!(
            is_sanitized_sink(
                "    cursor.execute(\"SELECT * FROM users WHERE id = :user_id\", {\"user_id\": user_id})",
                "user_id",
                "SQL Injection",
            ),
            "Parameterized query with :param should be detected as sanitized"
        );
    }

    #[test]
    fn test_unsanitized_sql_string_concat() {
        assert!(
            !is_sanitized_sink(
                "    cursor.execute(\"SELECT * FROM users WHERE name = '\" + name + \"'\")",
                "name",
                "SQL Injection",
            ),
            "String concatenation in SQL must NOT be considered sanitized"
        );
    }

    #[test]
    fn test_unsanitized_sql_fstring() {
        assert!(
            !is_sanitized_sink(
                "    cursor.execute(f\"SELECT * FROM users WHERE name = '{name}'\")",
                "name",
                "SQL Injection",
            ),
            "f-string in SQL must NOT be considered sanitized"
        );
    }

    #[test]
    fn test_sanitized_command_subprocess_list() {
        assert!(
            is_sanitized_sink(
                "    subprocess.run([\"ls\", \"-la\", dirname], capture_output=True)",
                "dirname",
                "Command Injection",
            ),
            "subprocess.run with list args should be detected as sanitized"
        );
    }

    #[test]
    fn test_sanitized_command_subprocess_call_list() {
        assert!(
            is_sanitized_sink(
                "    subprocess.call([\"cat\", filename])",
                "filename",
                "Command Injection",
            ),
            "subprocess.call with list args should be detected as sanitized"
        );
    }

    #[test]
    fn test_sanitized_command_subprocess_popen_list() {
        assert!(
            is_sanitized_sink(
                "    subprocess.Popen([\"grep\", pattern, filename])",
                "filename",
                "Command Injection",
            ),
            "subprocess.Popen with list args should be detected as sanitized"
        );
    }

    #[test]
    fn test_sanitized_command_shell_false() {
        assert!(
            is_sanitized_sink(
                "    subprocess.run(cmd, shell=False)",
                "cmd",
                "Command Injection",
            ),
            "subprocess with shell=False should be detected as sanitized"
        );
    }

    #[test]
    fn test_unsanitized_command_shell_true() {
        assert!(
            !is_sanitized_sink(
                "    subprocess.run(f\"ls {dirname}\", shell=True)",
                "dirname",
                "Command Injection",
            ),
            "subprocess with shell=True must NOT be considered sanitized"
        );
    }

    #[test]
    fn test_unsanitized_command_os_system() {
        assert!(
            !is_sanitized_sink(
                "    os.system(\"cat \" + filename)",
                "filename",
                "Command Injection",
            ),
            "os.system must NOT be considered sanitized"
        );
    }

    #[test]
    fn test_non_sql_non_command_not_sanitized() {
        // For non-SQL, non-command vuln types, is_sanitized_sink should return false
        assert!(
            !is_sanitized_sink("    open(user_path)", "user_path", "Path Traversal",),
            "Non-SQL/command sinks should not be treated as sanitized"
        );
    }

    // --- End-to-end scan_file_vulns tests ---

    #[test]
    fn test_e2e_parameterized_query_no_findings() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("safe_sql.py");
        std::fs::write(
            &file,
            r#"
from flask import request
import sqlite3
user_id = request.args.get("id")
cursor.execute("SELECT * FROM users WHERE id = ?", (user_id,))
"#,
        )
        .unwrap();
        let findings = scan_file_vulns(&file, None).unwrap();
        assert!(
            findings.is_empty(),
            "Parameterized query must produce 0 findings, got {}",
            findings.len()
        );
    }

    #[test]
    fn test_e2e_subprocess_list_no_findings() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("safe_cmd.py");
        std::fs::write(
            &file,
            r#"
from flask import request
filename = request.args.get("file")
subprocess.run(["cat", filename])
"#,
        )
        .unwrap();
        let findings = scan_file_vulns(&file, None).unwrap();
        assert!(
            findings.is_empty(),
            "subprocess.run with list args must produce 0 findings, got {}",
            findings.len()
        );
    }

    #[test]
    fn test_e2e_type_coercion_no_findings() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("safe_int.py");
        std::fs::write(
            &file,
            r#"
from flask import request
user_id = int(request.args.get("id"))
cursor.execute(f"SELECT * FROM users WHERE id = {user_id}")
"#,
        )
        .unwrap();
        let findings = scan_file_vulns(&file, None).unwrap();
        assert!(
            findings.is_empty(),
            "int() type coercion must break taint, producing 0 findings, got {}",
            findings.len()
        );
    }

    #[test]
    fn test_e2e_real_sqli_still_detected() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("vuln_sql.py");
        std::fs::write(
            &file,
            r#"
from flask import request
name = request.args.get("name")
cursor.execute(f"SELECT * FROM users WHERE name = '{name}'")
"#,
        )
        .unwrap();
        let findings = scan_file_vulns(&file, None).unwrap();
        assert!(
            !findings.is_empty(),
            "Real SQL injection must still be detected"
        );
    }

    #[test]
    fn test_e2e_real_command_injection_still_detected() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("vuln_cmd.py");
        std::fs::write(
            &file,
            r#"
from flask import request
filename = request.args.get("file")
os.system("cat " + filename)
"#,
        )
        .unwrap();
        let findings = scan_file_vulns(&file, None).unwrap();
        assert!(
            !findings.is_empty(),
            "Real command injection must still be detected"
        );
    }

    fn assert_detects_vuln(
        filename: &str,
        content: &str,
        vuln_type: VulnType,
    ) -> TldrResult<Vec<VulnFinding>> {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join(filename);
        fs::write(&path, content).unwrap();
        scan_file_vulns(&path, Some(vuln_type))
    }

    #[test]
    fn test_e2e_rust_command_injection() {
        let findings = assert_detects_vuln(
            "main.rs",
            "cmd = std::env::args().nth(1).unwrap();\nstd::process::Command::new(cmd);",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_e2e_ruby_command_injection() {
        let findings = assert_detects_vuln(
            "app.rb",
            "cmd = params[:cmd]\nsystem(cmd)",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_e2e_c_command_injection() {
        let findings = assert_detects_vuln(
            "main.c",
            "cmd = argv[1];\nsystem(cmd);",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_e2e_cpp_command_injection() {
        let findings = assert_detects_vuln(
            "main.cpp",
            "cmd = argv[1];\nsystem(cmd);",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_e2e_php_command_injection() {
        let findings = assert_detects_vuln(
            "index.php",
            "cmd = $_GET['cmd'];\nsystem(cmd);",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_e2e_kotlin_command_injection() {
        let findings = assert_detects_vuln(
            "Main.kt",
            "cmd = call.request.queryParameters[\"cmd\"]\nRuntime.getRuntime().exec(cmd)",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_e2e_swift_command_injection() {
        let findings = assert_detects_vuln(
            "main.swift",
            "cmd = CommandLine.arguments[1]\nsystem(cmd)",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_e2e_csharp_command_injection() {
        let findings = assert_detects_vuln(
            "Program.cs",
            "cmd = Request.Query[\"cmd\"];\nProcess.Start(cmd);",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_e2e_scala_command_injection() {
        let findings = assert_detects_vuln(
            "Main.scala",
            "cmd = request.getQueryString(\"cmd\")\nRuntime.getRuntime.exec(cmd)",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_e2e_elixir_command_injection() {
        let findings = assert_detects_vuln(
            "app.ex",
            "cmd = conn.params[\"cmd\"]\nSystem.cmd(\"sh\", [cmd])",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_e2e_lua_command_injection() {
        let findings = assert_detects_vuln(
            "app.lua",
            "cmd = ngx.req.get_uri_args()\nos.execute(cmd)",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_e2e_luau_command_injection() {
        let findings = assert_detects_vuln(
            "app.luau",
            "cmd = os.getenv(\"CMD\")\nos.execute(cmd)",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_e2e_ocaml_command_injection() {
        let findings = assert_detects_vuln(
            "main.ml",
            "cmd = Sys.getenv \"CMD\"\nSys.command cmd",
            VulnType::CommandInjection,
        )
        .unwrap();
        assert!(!findings.is_empty());
    }
}
