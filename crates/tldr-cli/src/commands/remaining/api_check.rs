//! API Check command - Detect API misuse patterns
//!
//! Analyzes Python code for common API misuse patterns:
//! - Timeout issues (requests.get without timeout)
//! - Bare except clauses (catching all exceptions)
//! - Weak crypto (MD5, SHA1 for security purposes)
//! - Unclosed resources (files not using context managers)
//!
//! # Example
//!
//! ```bash
//! tldr api-check src/
//! tldr api-check src/main.py --category crypto
//! tldr api-check src/ --severity high --format text
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;
use regex::Regex;
use walkdir::WalkDir;

use super::error::RemainingError;
use super::types::{
    APICheckReport, APICheckSummary, APIRule, MisuseCategory, MisuseFinding, MisuseSeverity,
};

use crate::output::OutputWriter;

// =============================================================================
// Constants
// =============================================================================

/// Maximum files to analyze in a directory
const MAX_DIRECTORY_FILES: u32 = 1000;

/// Maximum file size to analyze (10 MB)
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiLanguage {
    Python,
    Rust,
    Go,
    Java,
    JavaScript,
    TypeScript,
    C,
    Cpp,
    Ruby,
    Php,
    Kotlin,
    Swift,
    CSharp,
    Scala,
    Elixir,
    Lua,
    Luau,
    Ocaml,
}

#[derive(Clone, Copy)]
struct RegexRuleSpec {
    id: &'static str,
    name: &'static str,
    category: MisuseCategory,
    severity: MisuseSeverity,
    description: &'static str,
    correct_usage: &'static str,
    pattern: &'static str,
    api_call: &'static str,
    message: &'static str,
    fix_suggestion: &'static str,
}

impl RegexRuleSpec {
    fn rule(self) -> APIRule {
        APIRule {
            id: self.id.to_string(),
            name: self.name.to_string(),
            category: self.category,
            severity: self.severity,
            description: self.description.to_string(),
            correct_usage: self.correct_usage.to_string(),
        }
    }
}

const GO_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "GO001",
        name: "deprecated-ioutil-readfile",
        category: MisuseCategory::Resources,
        severity: MisuseSeverity::Low,
        description: "ioutil.ReadFile is deprecated and encourages unbounded whole-file reads",
        correct_usage: "Use os.ReadFile or stream with bufio.Scanner/Reader",
        pattern: r"\bioutil\.ReadFile\s*\(",
        api_call: "ioutil.ReadFile",
        message: "ioutil.ReadFile is deprecated and can load unbounded content into memory",
        fix_suggestion: "Use os.ReadFile for simple reads or bufio.Reader for bounded streaming",
    },
    RegexRuleSpec {
        id: "GO002",
        name: "http-get-without-timeout",
        category: MisuseCategory::Parameters,
        severity: MisuseSeverity::Medium,
        description: "http.Get uses the default client and provides no call-specific timeout",
        correct_usage: "Use an http.Client with Timeout or context-aware requests",
        pattern: r"\bhttp\.Get\s*\(",
        api_call: "http.Get",
        message: "http.Get without an explicit timeout can hang indefinitely",
        fix_suggestion: "Use an http.Client{Timeout: ...} or NewRequestWithContext",
    },
    RegexRuleSpec {
        id: "GO003",
        name: "exec-command",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "exec.Command is risky when arguments or executable names come from input",
        correct_usage: "Prefer direct library APIs or strictly validate allowed commands",
        pattern: r"\bexec\.Command\s*\(",
        api_call: "exec.Command",
        message: "exec.Command can enable command injection when fed user-controlled values",
        fix_suggestion: "Validate commands against an allowlist and avoid shell-like execution",
    },
    RegexRuleSpec {
        id: "GO004",
        name: "template-html-cast",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "template.HTML bypasses html/template escaping guarantees",
        correct_usage: "Pass plain strings to templates and let html/template escape them",
        pattern: r"\btemplate\.HTML\s*\(",
        api_call: "template.HTML",
        message: "template.HTML disables escaping and can introduce XSS",
        fix_suggestion: "Remove the cast and rely on html/template auto-escaping",
    },
    RegexRuleSpec {
        id: "GO005",
        name: "sql-query-without-context",
        category: MisuseCategory::CallOrder,
        severity: MisuseSeverity::Medium,
        description: "sql.DB.Query lacks cancellation and timeout propagation compared with QueryContext",
        correct_usage: "Use db.QueryContext(ctx, query, args...)",
        pattern: r"\bsql\.Query\s*\(",
        api_call: "sql.Query",
        message: "sql.Query omits context-driven cancellation and timeout handling",
        fix_suggestion: "Use QueryContext/ExecContext with a bounded context",
    },
];

const JAVA_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "JV001",
        name: "string-comparison-with-double-equals",
        category: MisuseCategory::CallOrder,
        severity: MisuseSeverity::Medium,
        description: "Using == on strings compares references instead of values",
        correct_usage: "Use value.equals(other) or Objects.equals(a, b)",
        pattern: r#"(?:".*"|\b\w+\b)\s*==\s*(?:".*"|\b\w+\b)"#,
        api_call: "==",
        message: "String comparison with == checks reference identity, not value equality",
        fix_suggestion: "Use .equals(...) or Objects.equals(...) for string values",
    },
    RegexRuleSpec {
        id: "JV002",
        name: "runtime-exec",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "Runtime.exec is dangerous with dynamic input and hard to sandbox correctly",
        correct_usage: "Use structured APIs or a ProcessBuilder with validated arguments",
        pattern: r"\bRuntime\.getRuntime\(\)\.exec\s*\(",
        api_call: "Runtime.exec",
        message: "Runtime.exec is a common command injection footgun",
        fix_suggestion: "Prefer library APIs or tightly validated ProcessBuilder arguments",
    },
    RegexRuleSpec {
        id: "JV003",
        name: "objectinputstream-deserialization",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "ObjectInputStream on untrusted data can trigger unsafe deserialization gadgets",
        correct_usage: "Use safer formats like JSON with explicit schemas",
        pattern: r"\bnew\s+ObjectInputStream\s*\(",
        api_call: "ObjectInputStream",
        message: "ObjectInputStream enables unsafe native Java deserialization",
        fix_suggestion: "Replace native object deserialization with a schema-driven format",
    },
    RegexRuleSpec {
        id: "JV004",
        name: "create-statement",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::Medium,
        description: "createStatement often leads to string-built SQL instead of prepared statements",
        correct_usage: "Use prepareStatement with placeholders",
        pattern: r"\bcreateStatement\s*\(",
        api_call: "createStatement",
        message: "createStatement encourages dynamic SQL and weak parameter handling",
        fix_suggestion: "Use prepareStatement with bound parameters",
    },
    RegexRuleSpec {
        id: "JV005",
        name: "system-gc-call",
        category: MisuseCategory::Resources,
        severity: MisuseSeverity::Low,
        description: "System.gc() is usually a performance smell and not a reliable memory fix",
        correct_usage: "Remove manual GC triggers and profile allocations instead",
        pattern: r"\bSystem\.gc\s*\(",
        api_call: "System.gc",
        message: "System.gc() is an unreliable manual GC hint and often harms latency",
        fix_suggestion: "Remove the call and fix the underlying allocation or lifetime issue",
    },
];

const JAVASCRIPT_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "JS001",
        name: "loose-equality",
        category: MisuseCategory::CallOrder,
        severity: MisuseSeverity::Medium,
        description: "Loose equality allows coercions that frequently hide correctness bugs",
        correct_usage: "Use === / !== except in deliberately reviewed coercion cases",
        pattern: r"\s==\s|\s!=\s",
        api_call: "==",
        message: "Loose equality can coerce values unexpectedly",
        fix_suggestion: "Use === or !== and handle explicit type conversion",
    },
    RegexRuleSpec {
        id: "JS002",
        name: "parseint-without-radix",
        category: MisuseCategory::Parameters,
        severity: MisuseSeverity::Low,
        description: "parseInt without a radix is ambiguous and less explicit than required",
        correct_usage: "Use parseInt(value, 10)",
        pattern: r"\bparseInt\s*\(\s*[^,\)]+\)",
        api_call: "parseInt",
        message: "parseInt called without an explicit radix",
        fix_suggestion: "Pass a radix explicitly, usually parseInt(value, 10)",
    },
    RegexRuleSpec {
        id: "JS003",
        name: "json-parse-without-guard",
        category: MisuseCategory::ErrorHandling,
        severity: MisuseSeverity::Low,
        description: "JSON.parse throws on malformed input and should usually be guarded",
        correct_usage: "Wrap JSON.parse in try/catch when input is not fully trusted",
        pattern: r"\bJSON\.parse\s*\(",
        api_call: "JSON.parse",
        message: "JSON.parse can throw and should be guarded for untrusted input",
        fix_suggestion: "Use try/catch or validated parsing for untrusted payloads",
    },
    RegexRuleSpec {
        id: "JS004",
        name: "document-write",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "document.write is legacy, brittle, and can inject unsanitized HTML",
        correct_usage: "Use DOM APIs like textContent/appendChild instead",
        pattern: r"\bdocument\.write(?:ln)?\s*\(",
        api_call: "document.write",
        message: "document.write is unsafe and can enable XSS",
        fix_suggestion: "Use safe DOM APIs instead of writing raw HTML strings",
    },
    RegexRuleSpec {
        id: "JS005",
        name: "eval-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "eval executes dynamic code and should be avoided",
        correct_usage: "Use structured data parsing or explicit dispatch tables",
        pattern: r"\beval\s*\(",
        api_call: "eval",
        message: "eval executes dynamic code and creates major security risk",
        fix_suggestion: "Replace eval with data parsing or explicit function dispatch",
    },
];

const TYPESCRIPT_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "TS001",
        name: "loose-equality",
        category: MisuseCategory::CallOrder,
        severity: MisuseSeverity::Medium,
        description: "Loose equality allows coercions that frequently hide correctness bugs",
        correct_usage: "Use === / !== except in deliberately reviewed coercion cases",
        pattern: r"\s==\s|\s!=\s",
        api_call: "==",
        message: "Loose equality can coerce values unexpectedly",
        fix_suggestion: "Use === or !== and handle explicit type conversion",
    },
    RegexRuleSpec {
        id: "TS002",
        name: "parseint-without-radix",
        category: MisuseCategory::Parameters,
        severity: MisuseSeverity::Low,
        description: "parseInt without a radix is ambiguous and less explicit than required",
        correct_usage: "Use parseInt(value, 10)",
        pattern: r"\bparseInt\s*\(\s*[^,\)]+\)",
        api_call: "parseInt",
        message: "parseInt called without an explicit radix",
        fix_suggestion: "Pass a radix explicitly, usually parseInt(value, 10)",
    },
    RegexRuleSpec {
        id: "TS003",
        name: "json-parse-without-guard",
        category: MisuseCategory::ErrorHandling,
        severity: MisuseSeverity::Low,
        description: "JSON.parse throws on malformed input and should usually be guarded",
        correct_usage: "Wrap JSON.parse in try/catch when input is not fully trusted",
        pattern: r"\bJSON\.parse\s*\(",
        api_call: "JSON.parse",
        message: "JSON.parse can throw and should be guarded for untrusted input",
        fix_suggestion: "Use try/catch or validated parsing for untrusted payloads",
    },
    RegexRuleSpec {
        id: "TS004",
        name: "document-write",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "document.write is legacy, brittle, and can inject unsanitized HTML",
        correct_usage: "Use DOM APIs like textContent/appendChild instead",
        pattern: r"\bdocument\.write(?:ln)?\s*\(",
        api_call: "document.write",
        message: "document.write is unsafe and can enable XSS",
        fix_suggestion: "Use safe DOM APIs instead of writing raw HTML strings",
    },
    RegexRuleSpec {
        id: "TS005",
        name: "eval-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "eval executes dynamic code and should be avoided",
        correct_usage: "Use structured data parsing or explicit dispatch tables",
        pattern: r"\beval\s*\(",
        api_call: "eval",
        message: "eval executes dynamic code and creates major security risk",
        fix_suggestion: "Replace eval with data parsing or explicit function dispatch",
    },
];

const C_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "C001",
        name: "gets-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "gets cannot bound input and has been removed from the standard library",
        correct_usage: "Use fgets with an explicit buffer length",
        pattern: r"\bgets\s*\(",
        api_call: "gets",
        message: "gets is inherently unsafe and enables buffer overflows",
        fix_suggestion: "Use fgets(buffer, size, stdin) or another bounded API",
    },
    RegexRuleSpec {
        id: "C002",
        name: "strcpy-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "strcpy performs unbounded copies and easily overflows buffers",
        correct_usage: "Use snprintf, strlcpy, or explicit bounds checks",
        pattern: r"\bstrcpy\s*\(",
        api_call: "strcpy",
        message: "strcpy performs an unbounded copy",
        fix_suggestion: "Replace strcpy with a bounded copy strategy",
    },
    RegexRuleSpec {
        id: "C003",
        name: "sprintf-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "sprintf writes formatted data without a size bound",
        correct_usage: "Use snprintf with the destination buffer size",
        pattern: r"\bsprintf\s*\(",
        api_call: "sprintf",
        message: "sprintf can overflow fixed-size buffers",
        fix_suggestion: "Use snprintf(buffer, size, ...) instead",
    },
    RegexRuleSpec {
        id: "C004",
        name: "scanf-string-without-width",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "scanf with %s and no width limit can overflow the destination buffer",
        correct_usage: "Provide a width specifier or use fgets",
        pattern: r#"\bscanf\s*\(\s*"%s"#,
        api_call: "scanf",
        message: "scanf(\"%s\") reads unbounded input into a buffer",
        fix_suggestion: "Add a width limit or use fgets plus parsing",
    },
    RegexRuleSpec {
        id: "C005",
        name: "system-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "system executes a shell command and is dangerous with dynamic input",
        correct_usage: "Use execve-family APIs with validated arguments where possible",
        pattern: r"\bsystem\s*\(",
        api_call: "system",
        message: "system executes a shell and is a common command injection vector",
        fix_suggestion: "Avoid shell execution or tightly validate the command source",
    },
];

const CPP_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "CPP001",
        name: "strcpy-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "strcpy performs unbounded copies and easily overflows buffers",
        correct_usage: "Use std::string, snprintf, or another bounded copy strategy",
        pattern: r"\bstrcpy\s*\(",
        api_call: "strcpy",
        message: "strcpy performs an unbounded copy",
        fix_suggestion: "Use std::string or a bounded copy API instead",
    },
    RegexRuleSpec {
        id: "CPP002",
        name: "sprintf-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "sprintf writes formatted data without a size bound",
        correct_usage: "Use snprintf or std::format into a bounded container",
        pattern: r"\bsprintf\s*\(",
        api_call: "sprintf",
        message: "sprintf can overflow fixed-size buffers",
        fix_suggestion: "Use snprintf or a safer formatting abstraction",
    },
    RegexRuleSpec {
        id: "CPP003",
        name: "auto-ptr",
        category: MisuseCategory::Resources,
        severity: MisuseSeverity::Medium,
        description: "std::auto_ptr is obsolete and has broken transfer semantics",
        correct_usage: "Use std::unique_ptr or std::shared_ptr",
        pattern: r"\bstd::auto_ptr\s*<",
        api_call: "std::auto_ptr",
        message: "std::auto_ptr is obsolete and unsafe by modern ownership standards",
        fix_suggestion: "Replace std::auto_ptr with std::unique_ptr or std::shared_ptr",
    },
    RegexRuleSpec {
        id: "CPP004",
        name: "raw-new",
        category: MisuseCategory::Resources,
        severity: MisuseSeverity::Medium,
        description: "Raw new often leads to leaks and exception-safety issues",
        correct_usage: "Use std::make_unique or stack allocation where possible",
        pattern: r"\bnew\s+\w",
        api_call: "new",
        message: "Raw new makes ownership and exception safety harder to reason about",
        fix_suggestion: "Use std::make_unique, containers, or stack allocation",
    },
    RegexRuleSpec {
        id: "CPP005",
        name: "system-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "system executes a shell command and is dangerous with dynamic input",
        correct_usage: "Use direct process APIs with validated arguments when possible",
        pattern: r"(?:\bstd::)?system\s*\(",
        api_call: "system",
        message: "system executes a shell and is a common command injection vector",
        fix_suggestion: "Avoid shell execution or tightly validate all command components",
    },
];

const RUBY_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "RB001",
        name: "eval-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "eval executes dynamic Ruby code and should be avoided",
        correct_usage: "Use explicit dispatch or data parsing instead of dynamic code execution",
        pattern: r"\beval\s*\(",
        api_call: "eval",
        message: "eval executes dynamic code and creates major security risk",
        fix_suggestion: "Replace eval with explicit dispatch or structured parsing",
    },
    RegexRuleSpec {
        id: "RB002",
        name: "dynamic-send",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::Medium,
        description: "send can invoke arbitrary methods when fed untrusted method names",
        correct_usage: "Use public_send on a strict allowlist of method names",
        pattern: r"\.send\s*\(",
        api_call: "send",
        message: "send can dispatch to unsafe or unexpected methods",
        fix_suggestion: "Use public_send with a reviewed allowlist",
    },
    RegexRuleSpec {
        id: "RB003",
        name: "system-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "system executes a shell command and is dangerous with interpolated input",
        correct_usage: "Use array-form process APIs with validated arguments",
        pattern: r"\bsystem\s*\(",
        api_call: "system",
        message: "system is a common command injection footgun",
        fix_suggestion: "Avoid shell execution or pass validated argv-style arguments",
    },
    RegexRuleSpec {
        id: "RB004",
        name: "yaml-load",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "YAML.load can instantiate arbitrary objects from untrusted input",
        correct_usage: "Use YAML.safe_load with permitted classes",
        pattern: r"\bYAML\.load\s*\(",
        api_call: "YAML.load",
        message: "YAML.load can deserialize unsafe objects",
        fix_suggestion: "Use YAML.safe_load and restrict allowed classes",
    },
    RegexRuleSpec {
        id: "RB005",
        name: "marshal-load",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "Marshal.load on untrusted data is unsafe deserialization",
        correct_usage: "Use JSON or another safe, schema-checked format",
        pattern: r"\bMarshal\.load\s*\(",
        api_call: "Marshal.load",
        message: "Marshal.load performs unsafe native deserialization",
        fix_suggestion: "Replace Marshal.load with a safer serialization format",
    },
];

const PHP_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "PH001",
        name: "deprecated-mysql-functions",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "mysql_* APIs are removed and encourage unsafe query construction",
        correct_usage: "Use PDO or mysqli with prepared statements",
        pattern: r"\bmysql_[a-z_]+\s*\(",
        api_call: "mysql_*",
        message: "mysql_* functions are removed and unsafe by modern standards",
        fix_suggestion: "Migrate to PDO or mysqli prepared statements",
    },
    RegexRuleSpec {
        id: "PH002",
        name: "extract-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::Medium,
        description: "extract pollutes local scope and can overwrite important variables",
        correct_usage: "Read array keys explicitly instead of splatting them into scope",
        pattern: r"\bextract\s*\(",
        api_call: "extract",
        message: "extract can overwrite local variables and hide data flow",
        fix_suggestion: "Assign required keys explicitly instead of using extract",
    },
    RegexRuleSpec {
        id: "PH003",
        name: "eval-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "eval executes dynamic PHP code and should be avoided",
        correct_usage: "Use explicit dispatch or data parsing instead of dynamic code execution",
        pattern: r"\beval\s*\(",
        api_call: "eval",
        message: "eval executes dynamic code and creates major security risk",
        fix_suggestion: "Replace eval with explicit dispatch or structured parsing",
    },
    RegexRuleSpec {
        id: "PH004",
        name: "variable-variables",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::Medium,
        description: "Variable variables make scope mutation hard to reason about",
        correct_usage: "Use associative arrays or explicit variables instead",
        pattern: r"\$\$[A-Za-z_]",
        api_call: "$$",
        message: "Variable variables obscure data flow and can enable unsafe access patterns",
        fix_suggestion: "Use an array/map or explicit variable names instead",
    },
    RegexRuleSpec {
        id: "PH005",
        name: "unserialize-call",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "unserialize on untrusted data can trigger object injection chains",
        correct_usage: "Use json_decode or a safer schema-checked format",
        pattern: r"\bunserialize\s*\(",
        api_call: "unserialize",
        message: "unserialize enables unsafe object deserialization",
        fix_suggestion: "Replace unserialize with json_decode or a safe serializer",
    },
];

const KOTLIN_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "KT001",
        name: "force-unwrapped-null",
        category: MisuseCategory::ErrorHandling,
        severity: MisuseSeverity::Medium,
        description: "!! converts nullable values into runtime crashes",
        correct_usage: "Use safe calls, let, requireNotNull, or explicit branching",
        pattern: r"!!",
        api_call: "!!",
        message: "!! will throw NullPointerException on null values",
        fix_suggestion: "Use safe calls or explicit null handling instead of !!",
    },
    RegexRuleSpec {
        id: "KT002",
        name: "lateinit-var",
        category: MisuseCategory::ErrorHandling,
        severity: MisuseSeverity::Low,
        description: "lateinit shifts initialization failures to runtime",
        correct_usage: "Prefer constructor injection or nullable/state wrappers",
        pattern: r"\blateinit\s+var\b",
        api_call: "lateinit",
        message: "lateinit can fail at runtime if the property is read before initialization",
        fix_suggestion: "Prefer constructor injection or explicit nullable state",
    },
    RegexRuleSpec {
        id: "KT003",
        name: "globalscope-launch",
        category: MisuseCategory::Concurrency,
        severity: MisuseSeverity::Medium,
        description: "GlobalScope.launch escapes structured concurrency and leaks work",
        correct_usage: "Launch from a lifecycle-bound CoroutineScope",
        pattern: r"\bGlobalScope\.launch\s*\(",
        api_call: "GlobalScope.launch",
        message: "GlobalScope.launch detaches work from structured concurrency",
        fix_suggestion: "Use a lifecycle-bound CoroutineScope instead",
    },
    RegexRuleSpec {
        id: "KT004",
        name: "runtime-exec",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "Runtime.exec is dangerous with dynamic input and hard to sandbox correctly",
        correct_usage: "Use structured APIs or strictly validated ProcessBuilder arguments",
        pattern: r"\bRuntime\.getRuntime\(\)\.exec\s*\(",
        api_call: "Runtime.exec",
        message: "Runtime.exec is a common command injection footgun",
        fix_suggestion: "Prefer library APIs or tightly validated ProcessBuilder arguments",
    },
    RegexRuleSpec {
        id: "KT005",
        name: "thread-sleep",
        category: MisuseCategory::Concurrency,
        severity: MisuseSeverity::Low,
        description: "Thread.sleep blocks threads directly and is usually wrong in coroutine-based code",
        correct_usage: "Use delay(...) in coroutines or higher-level scheduling",
        pattern: r"\bThread\.sleep\s*\(",
        api_call: "Thread.sleep",
        message: "Thread.sleep blocks the current thread directly",
        fix_suggestion: "Use delay(...) or a proper scheduler instead",
    },
];

const SWIFT_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "SW001",
        name: "forced-cast",
        category: MisuseCategory::ErrorHandling,
        severity: MisuseSeverity::Medium,
        description: "as! crashes at runtime when the cast fails",
        correct_usage: "Use as? with conditional handling",
        pattern: r"\bas!\b",
        api_call: "as!",
        message: "Forced casts crash when the runtime type is different",
        fix_suggestion: "Use as? and handle the nil case explicitly",
    },
    RegexRuleSpec {
        id: "SW002",
        name: "forced-try",
        category: MisuseCategory::ErrorHandling,
        severity: MisuseSeverity::Medium,
        description: "try! crashes when the call throws",
        correct_usage: "Use do/catch or try? with explicit fallback",
        pattern: r"\btry!\b",
        api_call: "try!",
        message: "try! crashes the process on thrown errors",
        fix_suggestion: "Use do/catch or try? and handle failure explicitly",
    },
    RegexRuleSpec {
        id: "SW003",
        name: "force-unwrap",
        category: MisuseCategory::ErrorHandling,
        severity: MisuseSeverity::Medium,
        description: "Force unwrapping optionals crashes at runtime on nil",
        correct_usage: "Use if let, guard let, or nil-coalescing",
        pattern: r"\b[A-Za-z_][A-Za-z0-9_]*!",
        api_call: "!",
        message: "Force unwraps crash when the optional is nil",
        fix_suggestion: "Use optional binding or nil-coalescing instead of force unwraps",
    },
    RegexRuleSpec {
        id: "SW004",
        name: "nskeyedunarchiver",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "Legacy NSKeyedUnarchiver APIs on untrusted data are unsafe",
        correct_usage: "Use secure decoding APIs with requiresSecureCoding",
        pattern: r"\bNSKeyedUnarchiver\.unarchiveObject",
        api_call: "NSKeyedUnarchiver",
        message: "Legacy unarchiving can deserialize unexpected object graphs",
        fix_suggestion: "Use secure coding APIs and schema-checked decoding",
    },
    RegexRuleSpec {
        id: "SW005",
        name: "fatalerror-call",
        category: MisuseCategory::ErrorHandling,
        severity: MisuseSeverity::Low,
        description: "fatalError terminates the process and is risky outside clearly impossible states",
        correct_usage: "Return/throw recoverable errors where possible",
        pattern: r"\bfatalError\s*\(",
        api_call: "fatalError",
        message: "fatalError terminates the process immediately",
        fix_suggestion: "Use recoverable error handling unless the state is truly unreachable",
    },
];

const CSHARP_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "CS001",
        name: "binaryformatter",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "BinaryFormatter is insecure and obsolete for untrusted data",
        correct_usage: "Use System.Text.Json or another safe serializer",
        pattern: r"\bBinaryFormatter\b",
        api_call: "BinaryFormatter",
        message: "BinaryFormatter is insecure and should not be used",
        fix_suggestion: "Use System.Text.Json or another safe serializer",
    },
    RegexRuleSpec {
        id: "CS002",
        name: "gc-collect",
        category: MisuseCategory::Resources,
        severity: MisuseSeverity::Low,
        description: "GC.Collect is rarely the right fix and often harms latency",
        correct_usage: "Remove manual GC triggers and profile the real allocation issue",
        pattern: r"\bGC\.Collect\s*\(",
        api_call: "GC.Collect",
        message: "GC.Collect is an unreliable manual GC hint and often harms performance",
        fix_suggestion: "Remove the call and fix the underlying allocation issue",
    },
    RegexRuleSpec {
        id: "CS003",
        name: "task-result",
        category: MisuseCategory::Concurrency,
        severity: MisuseSeverity::Medium,
        description: "Task.Result blocks synchronously and can deadlock async flows",
        correct_usage: "Use await instead of blocking on Task.Result",
        pattern: r"\.Result\b",
        api_call: "Task.Result",
        message: "Task.Result blocks synchronously and can deadlock async contexts",
        fix_suggestion: "Use await and keep the async chain asynchronous",
    },
    RegexRuleSpec {
        id: "CS004",
        name: "task-wait",
        category: MisuseCategory::Concurrency,
        severity: MisuseSeverity::Medium,
        description: "Task.Wait blocks synchronously and can deadlock async flows",
        correct_usage: "Use await or WhenAll/WhenAny instead of blocking waits",
        pattern: r"\.Wait\s*\(",
        api_call: "Task.Wait",
        message: "Task.Wait blocks synchronously and can deadlock async contexts",
        fix_suggestion: "Use await or asynchronous coordination primitives instead",
    },
    RegexRuleSpec {
        id: "CS005",
        name: "process-start",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "Process.Start is dangerous with untrusted paths or arguments",
        correct_usage: "Use strict allowlists and avoid shell execution semantics",
        pattern: r"\bProcess\.Start\s*\(",
        api_call: "Process.Start",
        message: "Process.Start can enable command injection with untrusted inputs",
        fix_suggestion: "Validate executable and arguments against a strict allowlist",
    },
];

const SCALA_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "SC001",
        name: "null-usage",
        category: MisuseCategory::ErrorHandling,
        severity: MisuseSeverity::Low,
        description: "null bypasses Scala's stronger option-based absence modeling",
        correct_usage: "Use Option instead of null",
        pattern: r"\bnull\b",
        api_call: "null",
        message: "null reintroduces runtime absence bugs into Scala code",
        fix_suggestion: "Use Option and explicit pattern matching instead",
    },
    RegexRuleSpec {
        id: "SC002",
        name: "asinstanceof-cast",
        category: MisuseCategory::ErrorHandling,
        severity: MisuseSeverity::Medium,
        description: "asInstanceOf crashes at runtime when the type assumption is wrong",
        correct_usage: "Use pattern matching or TypeTag/ClassTag-aware APIs",
        pattern: r"\basInstanceOf\[",
        api_call: "asInstanceOf",
        message: "asInstanceOf creates unchecked runtime casts",
        fix_suggestion: "Use pattern matching or safer typed abstractions",
    },
    RegexRuleSpec {
        id: "SC003",
        name: "await-result",
        category: MisuseCategory::Concurrency,
        severity: MisuseSeverity::Medium,
        description: "Await.result blocks threads and can collapse asynchronous throughput",
        correct_usage: "Compose futures asynchronously instead of blocking",
        pattern: r"\bAwait\.result\s*\(",
        api_call: "Await.result",
        message: "Await.result blocks threads and can create deadlocks or latency spikes",
        fix_suggestion: "Use map/flatMap/for-comprehensions instead of blocking",
    },
    RegexRuleSpec {
        id: "SC004",
        name: "mutable-collection",
        category: MisuseCategory::Concurrency,
        severity: MisuseSeverity::Low,
        description: "scala.collection.mutable structures are harder to reason about under concurrency",
        correct_usage: "Prefer immutable collections unless mutation is intentionally scoped",
        pattern: r"\bscala\.collection\.mutable\.",
        api_call: "scala.collection.mutable",
        message: "Mutable collections can hide shared-state bugs",
        fix_suggestion: "Prefer immutable collections or encapsulate mutation carefully",
    },
    RegexRuleSpec {
        id: "SC005",
        name: "sys-process",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "sys.process.Process executes external commands and is dangerous with input-derived values",
        correct_usage: "Use library APIs or validate commands and arguments against an allowlist",
        pattern: r"\bsys\.process\.Process\s*\(",
        api_call: "sys.process.Process",
        message: "sys.process.Process can enable command injection with untrusted input",
        fix_suggestion: "Avoid shell-style execution or strictly validate all command parts",
    },
];

const ELIXIR_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "EX001",
        name: "string-to-atom",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "String.to_atom on untrusted input can exhaust the VM atom table",
        correct_usage: "Use String.to_existing_atom only for reviewed values or keep strings",
        pattern: r"\bString\.to_atom\s*\(",
        api_call: "String.to_atom",
        message: "String.to_atom can permanently grow the atom table from user input",
        fix_suggestion: "Keep values as strings or use a reviewed to_existing_atom path",
    },
    RegexRuleSpec {
        id: "EX002",
        name: "code-eval-string",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "Code.eval_string executes dynamic Elixir code and should be avoided",
        correct_usage: "Use explicit dispatch or data parsing instead of dynamic evaluation",
        pattern: r"\bCode\.eval_string\s*\(",
        api_call: "Code.eval_string",
        message: "Code.eval_string executes dynamic code and is a major security risk",
        fix_suggestion: "Replace dynamic evaluation with explicit dispatch or parsing",
    },
    RegexRuleSpec {
        id: "EX003",
        name: "binary-to-term",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: ":erlang.binary_to_term on untrusted data is unsafe deserialization",
        correct_usage: "Use safe formats like JSON or term_to_binary only for trusted data",
        pattern: r":erlang\.binary_to_term\s*\(",
        api_call: ":erlang.binary_to_term",
        message: ":erlang.binary_to_term can deserialize unsafe terms from untrusted input",
        fix_suggestion: "Use a safer serialization format for external input",
    },
    RegexRuleSpec {
        id: "EX004",
        name: "file-read-bang",
        category: MisuseCategory::ErrorHandling,
        severity: MisuseSeverity::Low,
        description: "Bang file APIs raise instead of returning tagged tuples",
        correct_usage: "Prefer File.read/1 with explicit {:ok, data} / {:error, reason} handling",
        pattern: r"\bFile\.read!\s*\(",
        api_call: "File.read!",
        message: "File.read! raises on failure instead of returning a recoverable error",
        fix_suggestion: "Use File.read/1 and handle the returned tuple explicitly",
    },
    RegexRuleSpec {
        id: "EX005",
        name: "task-await-infinity",
        category: MisuseCategory::Concurrency,
        severity: MisuseSeverity::Medium,
        description: "Task.await with :infinity can stall callers indefinitely",
        correct_usage: "Use bounded timeouts and supervised retry/cancellation behavior",
        pattern: r"\bTask\.await\s*\([^,]+,\s*:infinity\s*\)",
        api_call: "Task.await",
        message: "Task.await(..., :infinity) can block forever",
        fix_suggestion: "Use a bounded timeout and explicit failure handling",
    },
];

const LUA_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "LU001",
        name: "implicit-global",
        category: MisuseCategory::CallOrder,
        severity: MisuseSeverity::Low,
        description: "Assigning without local leaks mutable globals and creates hidden coupling",
        correct_usage: "Declare locals explicitly with local name = ...",
        pattern: r"^[A-Za-z_][A-Za-z0-9_]*\s*=",
        api_call: "global assignment",
        message: "Implicit global assignment leaks state outside local scope",
        fix_suggestion: "Prefix the binding with local to keep scope explicit",
    },
    RegexRuleSpec {
        id: "LU002",
        name: "dynamic-load",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "load/loadstring execute dynamic Lua code and should be avoided",
        correct_usage: "Use structured parsing or explicit dispatch instead of dynamic evaluation",
        pattern: r"\b(?:loadstring|load)\s*\(",
        api_call: "load",
        message: "Dynamic code loading executes attacker-controlled Lua if fed untrusted input",
        fix_suggestion: "Replace dynamic evaluation with explicit dispatch or parsing",
    },
    RegexRuleSpec {
        id: "LU003",
        name: "os-execute",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "os.execute shells out and is dangerous with dynamic input",
        correct_usage: "Avoid shell execution or validate every command component",
        pattern: r"\bos\.execute\s*\(",
        api_call: "os.execute",
        message: "os.execute can enable command injection with untrusted input",
        fix_suggestion: "Avoid shelling out or strictly validate the command source",
    },
    RegexRuleSpec {
        id: "LU004",
        name: "io-popen",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "io.popen launches shell commands and should be treated as high risk",
        correct_usage: "Use safer process APIs or validate all command components",
        pattern: r"\bio\.popen\s*\(",
        api_call: "io.popen",
        message: "io.popen can enable command injection with untrusted input",
        fix_suggestion: "Avoid shell execution or validate every command component",
    },
    RegexRuleSpec {
        id: "LU005",
        name: "dofile-loadfile",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::Medium,
        description: "dofile/loadfile execute external files and are risky with user-controlled paths",
        correct_usage: "Validate file origins strictly before executing them",
        pattern: r"\b(?:dofile|loadfile)\s*\(",
        api_call: "dofile",
        message: "Executing external files is dangerous when the path is not fully trusted",
        fix_suggestion: "Avoid dynamic file execution or tightly validate trusted origins",
    },
];

const OCAML_RULE_SPECS: &[RegexRuleSpec] = &[
    RegexRuleSpec {
        id: "OC001",
        name: "marshal-from-string",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "Marshal.from_string on untrusted data is unsafe native deserialization",
        correct_usage: "Use a safe, schema-checked serialization format",
        pattern: r"\bMarshal\.from_string\b",
        api_call: "Marshal.from_string",
        message: "Marshal.from_string can deserialize unsafe values from untrusted input",
        fix_suggestion: "Use a safer serialization format for external input",
    },
    RegexRuleSpec {
        id: "OC002",
        name: "marshal-from-channel",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "Marshal.from_channel on untrusted data is unsafe native deserialization",
        correct_usage: "Use a safe, schema-checked serialization format",
        pattern: r"\bMarshal\.from_channel\b",
        api_call: "Marshal.from_channel",
        message: "Marshal.from_channel can deserialize unsafe values from untrusted input",
        fix_suggestion: "Use a safer serialization format for external input",
    },
    RegexRuleSpec {
        id: "OC003",
        name: "sys-command",
        category: MisuseCategory::Security,
        severity: MisuseSeverity::High,
        description: "Sys.command executes a shell command and is dangerous with dynamic input",
        correct_usage: "Prefer direct library APIs or validate allowed commands strictly",
        pattern: r"\bSys\.command\b",
        api_call: "Sys.command",
        message: "Sys.command can enable command injection with untrusted input",
        fix_suggestion: "Avoid shell execution or tightly validate the command source",
    },
    RegexRuleSpec {
        id: "OC004",
        name: "obj-magic",
        category: MisuseCategory::ErrorHandling,
        severity: MisuseSeverity::High,
        description: "Obj.magic bypasses the type system and can produce memory-unsound behavior",
        correct_usage: "Use typed abstractions or explicit variant handling",
        pattern: r"\bObj\.magic\b",
        api_call: "Obj.magic",
        message: "Obj.magic bypasses type safety and can create undefined behavior",
        fix_suggestion: "Refactor to a typed abstraction instead of coercing with Obj.magic",
    },
    RegexRuleSpec {
        id: "OC005",
        name: "open-in-out",
        category: MisuseCategory::Resources,
        severity: MisuseSeverity::Low,
        description: "open_in/open_out require explicit close calls and are easy to leak",
        correct_usage: "Use In_channel.with_open_* or Out_channel.with_open_* helpers",
        pattern: r"\b(?:open_in|open_out)\b",
        api_call: "open_in",
        message: "open_in/open_out require explicit close handling and are easy to leak",
        fix_suggestion: "Use with_open_* helpers to scope the channel lifetime",
    },
];

const ALL_API_LANGUAGES: &[ApiLanguage] = &[
    ApiLanguage::Python,
    ApiLanguage::Rust,
    ApiLanguage::Go,
    ApiLanguage::Java,
    ApiLanguage::JavaScript,
    ApiLanguage::TypeScript,
    ApiLanguage::C,
    ApiLanguage::Cpp,
    ApiLanguage::Ruby,
    ApiLanguage::Php,
    ApiLanguage::Kotlin,
    ApiLanguage::Swift,
    ApiLanguage::CSharp,
    ApiLanguage::Scala,
    ApiLanguage::Elixir,
    ApiLanguage::Lua,
    ApiLanguage::Luau,
    ApiLanguage::Ocaml,
];

// =============================================================================
// Rule Definitions
// =============================================================================

/// Built-in Python API misuse rules
fn python_rules() -> Vec<APIRule> {
    vec![
        APIRule {
            id: "PY001".to_string(),
            name: "missing-timeout".to_string(),
            category: MisuseCategory::Parameters,
            severity: MisuseSeverity::High,
            description: "requests.get/post/etc without timeout parameter can hang indefinitely"
                .to_string(),
            correct_usage: "requests.get(url, timeout=30)".to_string(),
        },
        APIRule {
            id: "PY002".to_string(),
            name: "bare-except".to_string(),
            category: MisuseCategory::ErrorHandling,
            severity: MisuseSeverity::Medium,
            description: "Bare except clause catches all exceptions including KeyboardInterrupt"
                .to_string(),
            correct_usage: "except Exception as e:".to_string(),
        },
        APIRule {
            id: "PY003".to_string(),
            name: "weak-hash-md5".to_string(),
            category: MisuseCategory::Crypto,
            severity: MisuseSeverity::High,
            description: "MD5 is cryptographically broken, don't use for security purposes"
                .to_string(),
            correct_usage: "hashlib.sha256() or bcrypt for passwords".to_string(),
        },
        APIRule {
            id: "PY004".to_string(),
            name: "weak-hash-sha1".to_string(),
            category: MisuseCategory::Crypto,
            severity: MisuseSeverity::High,
            description: "SHA1 is cryptographically weak, don't use for security purposes"
                .to_string(),
            correct_usage: "hashlib.sha256() or stronger".to_string(),
        },
        APIRule {
            id: "PY005".to_string(),
            name: "unclosed-file".to_string(),
            category: MisuseCategory::Resources,
            severity: MisuseSeverity::Medium,
            description: "File opened without context manager may not be properly closed"
                .to_string(),
            correct_usage: "with open(path) as f:".to_string(),
        },
        APIRule {
            id: "PY006".to_string(),
            name: "insecure-random".to_string(),
            category: MisuseCategory::Security,
            severity: MisuseSeverity::High,
            description: "random module is not cryptographically secure".to_string(),
            correct_usage: "secrets.token_bytes() or secrets.token_hex()".to_string(),
        },
    ]
}

/// Built-in Rust API misuse rules
fn rust_rules() -> Vec<APIRule> {
    vec![
        APIRule {
            id: "RS001".to_string(),
            name: "mutex-lock-unwrap".to_string(),
            category: MisuseCategory::Concurrency,
            severity: MisuseSeverity::Medium,
            description: "Mutex::lock().unwrap() can panic and amplify lock contention (CWE-833)"
                .to_string(),
            correct_usage:
                "Prefer try_lock()/error handling or explicit poison recovery instead of unwrap()"
                    .to_string(),
        },
        APIRule {
            id: "RS002".to_string(),
            name: "file-open-without-context".to_string(),
            category: MisuseCategory::ErrorHandling,
            severity: MisuseSeverity::Low,
            description:
                "File::open without contextual error mapping makes failures hard to triage"
                    .to_string(),
            correct_usage:
                "File::open(path).with_context(|| format!(\"opening {}\", path.display()))?"
                    .to_string(),
        },
        APIRule {
            id: "RS003".to_string(),
            name: "unbounded-with-capacity".to_string(),
            category: MisuseCategory::Resources,
            severity: MisuseSeverity::High,
            description:
                "Vec::with_capacity fed from unbounded input can cause memory exhaustion (CWE-770)"
                    .to_string(),
            correct_usage: "Clamp capacity input before allocation (e.g. min(user_len, MAX))"
                .to_string(),
        },
        APIRule {
            id: "RS004".to_string(),
            name: "detached-tokio-spawn".to_string(),
            category: MisuseCategory::Concurrency,
            severity: MisuseSeverity::Medium,
            description: "tokio::spawn without retaining JoinHandle risks silent task failures"
                .to_string(),
            correct_usage: "Store JoinHandle values and await/join them".to_string(),
        },
        APIRule {
            id: "RS005".to_string(),
            name: "hashmap-order-dependence".to_string(),
            category: MisuseCategory::CallOrder,
            severity: MisuseSeverity::Low,
            description:
                "HashMap iteration order is non-deterministic; relying on it can break logic"
                    .to_string(),
            correct_usage:
                "Collect keys and sort them, or use BTreeMap/IndexMap when stable order is required"
                    .to_string(),
        },
        APIRule {
            id: "RS006".to_string(),
            name: "clone-in-hot-loop".to_string(),
            category: MisuseCategory::Resources,
            severity: MisuseSeverity::Low,
            description: "clone() inside loop bodies can create avoidable allocation pressure"
                .to_string(),
            correct_usage: "Borrow or move values instead of cloning in tight loops".to_string(),
        },
    ]
}

fn regex_rule_specs_for_language(language: ApiLanguage) -> &'static [RegexRuleSpec] {
    match language {
        ApiLanguage::Python | ApiLanguage::Rust => &[],
        ApiLanguage::Go => GO_RULE_SPECS,
        ApiLanguage::Java => JAVA_RULE_SPECS,
        ApiLanguage::JavaScript => JAVASCRIPT_RULE_SPECS,
        ApiLanguage::TypeScript => TYPESCRIPT_RULE_SPECS,
        ApiLanguage::C => C_RULE_SPECS,
        ApiLanguage::Cpp => CPP_RULE_SPECS,
        ApiLanguage::Ruby => RUBY_RULE_SPECS,
        ApiLanguage::Php => PHP_RULE_SPECS,
        ApiLanguage::Kotlin => KOTLIN_RULE_SPECS,
        ApiLanguage::Swift => SWIFT_RULE_SPECS,
        ApiLanguage::CSharp => CSHARP_RULE_SPECS,
        ApiLanguage::Scala => SCALA_RULE_SPECS,
        ApiLanguage::Elixir => ELIXIR_RULE_SPECS,
        ApiLanguage::Lua | ApiLanguage::Luau => LUA_RULE_SPECS,
        ApiLanguage::Ocaml => OCAML_RULE_SPECS,
    }
}

fn all_api_languages() -> &'static [ApiLanguage] {
    ALL_API_LANGUAGES
}

// =============================================================================
// CLI Arguments
// =============================================================================

/// Detect API misuse patterns in code
///
/// Analyzes code for common API misuse patterns like missing timeouts,
/// bare except clauses, weak crypto usage, and unclosed resources.
///
/// # Example
///
/// ```bash
/// tldr api-check src/
/// tldr api-check src/main.py --category crypto
/// tldr api-check src/ --severity high
/// ```
#[derive(Debug, Args)]
pub struct ApiCheckArgs {
    /// File or directory to analyze (path to file or directory)
    #[arg(value_name = "path")]
    pub path: PathBuf,

    /// Filter by misuse category
    #[arg(long, value_delimiter = ',')]
    pub category: Option<Vec<MisuseCategory>>,

    /// Filter by minimum severity
    #[arg(long, value_delimiter = ',')]
    pub severity: Option<Vec<MisuseSeverity>>,

    /// Output file (optional, stdout if not specified)
    #[arg(long, short = 'O')]
    pub output: Option<PathBuf>,
}

impl ApiCheckArgs {
    /// Run the api-check command
    pub fn run(&self, format: crate::output::OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        writer.progress(&format!(
            "Checking {} for API misuse patterns...",
            self.path.display()
        ));

        // Validate path exists
        if !self.path.exists() {
            return Err(RemainingError::file_not_found(&self.path).into());
        }

        let all_rules_count = all_api_languages()
            .iter()
            .map(|language| rules_for_language(*language).len() as u32)
            .sum();

        // Collect files to analyze
        let files = collect_files(&self.path)?;
        writer.progress(&format!("Found {} files to analyze", files.len()));

        // Analyze each file
        let mut all_findings: Vec<MisuseFinding> = Vec::new();
        let mut files_scanned = 0u32;

        for file_path in &files {
            let Some(language) = detect_language(file_path) else {
                continue;
            };
            let rules = rules_for_language(language);
            match analyze_file(file_path, &rules, language) {
                Ok(findings) => {
                    all_findings.extend(findings);
                    files_scanned += 1;
                }
                Err(e) => {
                    writer.progress(&format!(
                        "Warning: Failed to analyze {}: {}",
                        file_path.display(),
                        e
                    ));
                }
            }
        }

        // Apply filters
        let filtered_findings = filter_findings(
            all_findings,
            self.category.as_deref(),
            self.severity.as_deref(),
        );

        // Build summary
        let summary = build_summary(&filtered_findings, files_scanned);

        // Build report
        let report = APICheckReport {
            findings: filtered_findings,
            summary,
            rules_applied: all_rules_count,
        };

        // Write output
        if let Some(ref output_path) = self.output {
            if writer.is_text() {
                let text = format_api_check_text(&report);
                fs::write(output_path, text)?;
            } else {
                let json = serde_json::to_string_pretty(&report)?;
                fs::write(output_path, json)?;
            }
        } else if writer.is_text() {
            let text = format_api_check_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

// =============================================================================
// File Collection
// =============================================================================

/// Collect supported source files from a path
fn collect_files(path: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    if path.is_file() {
        if is_supported_file(path) {
            files.push(path.to_path_buf());
        }
    } else if path.is_dir() {
        for entry in WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if files.len() >= MAX_DIRECTORY_FILES as usize {
                break;
            }

            let entry_path = entry.path();
            if entry_path.is_file() && is_supported_file(entry_path) {
                // Check file size
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

/// Check if a path has a supported extension.
fn is_supported_file(path: &Path) -> bool {
    detect_language(path).is_some()
}

pub(crate) fn detect_language(path: &Path) -> Option<ApiLanguage> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("py") => Some(ApiLanguage::Python),
        Some("rs") => Some(ApiLanguage::Rust),
        Some("go") => Some(ApiLanguage::Go),
        Some("java") => Some(ApiLanguage::Java),
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => Some(ApiLanguage::JavaScript),
        Some("ts") | Some("tsx") => Some(ApiLanguage::TypeScript),
        Some("c") | Some("h") => Some(ApiLanguage::C),
        Some("cpp") | Some("hpp") | Some("cc") | Some("cxx") => Some(ApiLanguage::Cpp),
        Some("rb") => Some(ApiLanguage::Ruby),
        Some("php") => Some(ApiLanguage::Php),
        Some("kt") | Some("kts") => Some(ApiLanguage::Kotlin),
        Some("swift") => Some(ApiLanguage::Swift),
        Some("cs") => Some(ApiLanguage::CSharp),
        Some("scala") => Some(ApiLanguage::Scala),
        Some("ex") | Some("exs") => Some(ApiLanguage::Elixir),
        Some("lua") => Some(ApiLanguage::Lua),
        Some("luau") => Some(ApiLanguage::Luau),
        Some("ml") | Some("mli") => Some(ApiLanguage::Ocaml),
        _ => None,
    }
}

pub(crate) fn rules_for_language(language: ApiLanguage) -> Vec<APIRule> {
    match language {
        ApiLanguage::Python => python_rules(),
        ApiLanguage::Rust => rust_rules(),
        _ => regex_rule_specs_for_language(language)
            .iter()
            .copied()
            .map(RegexRuleSpec::rule)
            .collect(),
    }
}

// =============================================================================
// Analysis Engine
// =============================================================================

/// Analyze a single file for API misuse
pub(crate) fn analyze_file(
    path: &Path,
    rules: &[APIRule],
    language: ApiLanguage,
) -> Result<Vec<MisuseFinding>> {
    let content = fs::read_to_string(path)?;
    let file_str = path.display().to_string();
    let mut findings = Vec::new();
    let mut prev_trimmed = String::new();
    let file_has_hashmap = matches!(language, ApiLanguage::Rust) && content.contains("HashMap");

    for (line_num, line) in content.lines().enumerate() {
        let line_number = (line_num + 1) as u32;
        let trimmed = line.trim();
        let rust_ctx = RustLineContext {
            file_has_hashmap,
            previous_line: prev_trimmed.as_str(),
            previous_is_loop: prev_trimmed.starts_with("for ")
                || prev_trimmed.starts_with("while "),
        };

        // Check each rule
        for rule in rules {
            if let Some(finding) =
                check_rule(rule, &file_str, line_number, line, language, &rust_ctx)
            {
                findings.push(finding);
            }
        }
        prev_trimmed = trimmed.to_string();
    }

    Ok(findings)
}

struct RustLineContext<'a> {
    file_has_hashmap: bool,
    previous_line: &'a str,
    previous_is_loop: bool,
}

/// Check a single rule against a line of code
fn check_rule(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
    language: ApiLanguage,
    rust_ctx: &RustLineContext<'_>,
) -> Option<MisuseFinding> {
    let trimmed = line_text.trim();

    // Skip comments
    if is_comment_line(trimmed, language) {
        return None;
    }

    match rule.id.as_str() {
        "PY001" => check_missing_timeout(rule, file, line, trimmed),
        "PY002" => check_bare_except(rule, file, line, trimmed),
        "PY003" => check_md5_usage(rule, file, line, trimmed),
        "PY004" => check_sha1_usage(rule, file, line, trimmed),
        "PY005" => check_unclosed_file(rule, file, line, trimmed),
        "PY006" => check_insecure_random(rule, file, line, trimmed),
        "RS001" => check_mutex_lock_unwrap(rule, file, line, trimmed),
        "RS002" => check_file_open_without_context(rule, file, line, trimmed),
        "RS003" => check_unbounded_with_capacity(rule, file, line, trimmed),
        "RS004" => check_detached_tokio_spawn(rule, file, line, trimmed),
        "RS005" => check_hashmap_order_dependence(rule, file, line, trimmed, rust_ctx),
        "RS006" => check_clone_in_hot_loop(rule, file, line, trimmed, rust_ctx),
        _ => check_regex_rule(rule, file, line, trimmed, language),
    }
}

fn is_comment_line(trimmed: &str, language: ApiLanguage) -> bool {
    match language {
        ApiLanguage::Python | ApiLanguage::Ruby | ApiLanguage::Elixir => trimmed.starts_with('#'),
        ApiLanguage::Rust
        | ApiLanguage::Go
        | ApiLanguage::Java
        | ApiLanguage::JavaScript
        | ApiLanguage::TypeScript
        | ApiLanguage::C
        | ApiLanguage::Cpp
        | ApiLanguage::Kotlin
        | ApiLanguage::Swift
        | ApiLanguage::CSharp
        | ApiLanguage::Scala => trimmed.starts_with("//"),
        ApiLanguage::Php => trimmed.starts_with("//") || trimmed.starts_with('#'),
        ApiLanguage::Lua | ApiLanguage::Luau => trimmed.starts_with("--"),
        ApiLanguage::Ocaml => trimmed.starts_with("(*"),
    }
}

fn check_regex_rule(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
    language: ApiLanguage,
) -> Option<MisuseFinding> {
    let spec = regex_rule_specs_for_language(language)
        .iter()
        .find(|spec| spec.id == rule.id)?;
    let regex = Regex::new(spec.pattern).ok()?;
    if !regex.is_match(line_text) {
        return None;
    }

    let column = regex.find(line_text).map(|m| m.start()).unwrap_or(0) as u32;
    Some(MisuseFinding {
        file: file.to_string(),
        line,
        column,
        rule: rule.clone(),
        api_call: spec.api_call.to_string(),
        message: spec.message.to_string(),
        fix_suggestion: spec.fix_suggestion.to_string(),
        code_context: line_text.to_string(),
    })
}

/// Check for requests without timeout
fn check_missing_timeout(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
) -> Option<MisuseFinding> {
    // Look for requests.get/post/put/delete/patch without timeout
    let request_patterns = [
        "requests.get(",
        "requests.post(",
        "requests.put(",
        "requests.delete(",
        "requests.patch(",
        "requests.head(",
        "requests.options(",
    ];

    for pattern in &request_patterns {
        if line_text.contains(pattern) && !line_text.contains("timeout") {
            let column = line_text.find(pattern).unwrap_or(0) as u32;
            return Some(MisuseFinding {
                file: file.to_string(),
                line,
                column,
                rule: rule.clone(),
                api_call: pattern.trim_end_matches('(').to_string(),
                message: format!(
                    "{} called without timeout parameter",
                    pattern.trim_end_matches('(')
                ),
                fix_suggestion: format!("Add timeout parameter: {}url, timeout=30)", pattern),
                code_context: line_text.to_string(),
            });
        }
    }

    None
}

/// Check for bare except clause
fn check_bare_except(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
) -> Option<MisuseFinding> {
    // Look for "except:" without an exception type
    // Match "except:" but not "except SomeException:" or "except Exception as e:"
    if line_text.starts_with("except:") || line_text.contains(" except:") {
        let column = line_text.find("except:").unwrap_or(0) as u32;
        return Some(MisuseFinding {
            file: file.to_string(),
            line,
            column,
            rule: rule.clone(),
            api_call: "except".to_string(),
            message: "Bare except clause catches all exceptions including KeyboardInterrupt and SystemExit".to_string(),
            fix_suggestion: "Use 'except Exception as e:' to catch only program exceptions".to_string(),
            code_context: line_text.to_string(),
        });
    }

    None
}

/// Check for MD5 usage
fn check_md5_usage(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
) -> Option<MisuseFinding> {
    // Look for hashlib.md5 usage
    if line_text.contains("hashlib.md5") || line_text.contains("md5(") {
        let column = line_text
            .find("hashlib.md5")
            .or_else(|| line_text.find("md5("))
            .unwrap_or(0) as u32;
        return Some(MisuseFinding {
            file: file.to_string(),
            line,
            column,
            rule: rule.clone(),
            api_call: "hashlib.md5".to_string(),
            message: "MD5 is cryptographically broken and should not be used for security purposes"
                .to_string(),
            fix_suggestion: "Use hashlib.sha256() or stronger. For passwords, use bcrypt or argon2"
                .to_string(),
            code_context: line_text.to_string(),
        });
    }

    None
}

/// Check for SHA1 usage
fn check_sha1_usage(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
) -> Option<MisuseFinding> {
    // Look for hashlib.sha1 usage
    if line_text.contains("hashlib.sha1") || line_text.contains("sha1(") {
        let column = line_text
            .find("hashlib.sha1")
            .or_else(|| line_text.find("sha1("))
            .unwrap_or(0) as u32;
        return Some(MisuseFinding {
            file: file.to_string(),
            line,
            column,
            rule: rule.clone(),
            api_call: "hashlib.sha1".to_string(),
            message: "SHA1 is cryptographically weak and should not be used for security purposes"
                .to_string(),
            fix_suggestion: "Use hashlib.sha256() or stronger".to_string(),
            code_context: line_text.to_string(),
        });
    }

    None
}

/// Check for unclosed file
fn check_unclosed_file(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
) -> Option<MisuseFinding> {
    // Look for "open(" that's not after "with "
    // This is a simplified check - a proper implementation would use AST
    if line_text.contains("open(")
        && !line_text.contains("with ")
        && !line_text.starts_with("with ")
    {
        // Check if it's an assignment (f = open(...))
        if line_text.contains("= open(") || line_text.contains("=open(") {
            let column = line_text.find("open(").unwrap_or(0) as u32;
            return Some(MisuseFinding {
                file: file.to_string(),
                line,
                column,
                rule: rule.clone(),
                api_call: "open".to_string(),
                message: "File opened without context manager may not be properly closed"
                    .to_string(),
                fix_suggestion: "Use 'with open(path) as f:' to ensure file is closed".to_string(),
                code_context: line_text.to_string(),
            });
        }
    }

    None
}

/// Check for insecure random usage
fn check_insecure_random(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
) -> Option<MisuseFinding> {
    // Look for random.* usage that might be for security
    let insecure_patterns = [
        "random.randint(",
        "random.random(",
        "random.choice(",
        "random.randrange(",
    ];

    // Only flag if it looks like it's being used for security
    // (contains words like token, secret, password, key)
    let security_indicators = ["token", "secret", "password", "key", "auth", "session"];

    for pattern in &insecure_patterns {
        if line_text.contains(pattern) {
            // Check if the line or nearby context suggests security use
            let line_lower = line_text.to_lowercase();
            for indicator in &security_indicators {
                if line_lower.contains(indicator) {
                    let column = line_text.find(pattern).unwrap_or(0) as u32;
                    return Some(MisuseFinding {
                        file: file.to_string(),
                        line,
                        column,
                        rule: rule.clone(),
                        api_call: pattern.trim_end_matches('(').to_string(),
                        message: format!(
                            "{} is not cryptographically secure, don't use for security purposes",
                            pattern.trim_end_matches('(')
                        ),
                        fix_suggestion:
                            "Use secrets.token_bytes() or secrets.token_hex() for security"
                                .to_string(),
                        code_context: line_text.to_string(),
                    });
                }
            }
        }
    }

    None
}

/// Check for poisoned mutex lock unwrap.
fn check_mutex_lock_unwrap(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
) -> Option<MisuseFinding> {
    if line_text.contains(".lock().unwrap()") {
        let column = line_text.find(".lock().unwrap()").unwrap_or(0) as u32;
        return Some(MisuseFinding {
            file: file.to_string(),
            line,
            column,
            rule: rule.clone(),
            api_call: "Mutex::lock".to_string(),
            message:
                "Mutex::lock().unwrap() can panic on poisoned locks and hide deadlock behavior"
                    .to_string(),
            fix_suggestion:
                "Handle lock errors explicitly (match/if let), or use try_lock with backoff"
                    .to_string(),
            code_context: line_text.to_string(),
        });
    }
    None
}

/// Check for File::open without context propagation.
fn check_file_open_without_context(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
) -> Option<MisuseFinding> {
    if line_text.contains("File::open(")
        && !line_text.contains(".context(")
        && !line_text.contains(".with_context(")
        && !line_text.contains("map_err(")
    {
        let column = line_text.find("File::open(").unwrap_or(0) as u32;
        return Some(MisuseFinding {
            file: file.to_string(),
            line,
            column,
            rule: rule.clone(),
            api_call: "File::open".to_string(),
            message: "File::open used without contextual error mapping".to_string(),
            fix_suggestion:
                "Wrap errors with context (with_context/context/map_err) before propagating"
                    .to_string(),
            code_context: line_text.to_string(),
        });
    }
    None
}

/// Check for capacity allocations sourced from unbounded input.
fn check_unbounded_with_capacity(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
) -> Option<MisuseFinding> {
    if line_text.contains("Vec::with_capacity(") {
        let line_lower = line_text.to_lowercase();
        let user_input_markers = [
            "input", "args", "user", "request", "len", "size",
        ];
        if user_input_markers.iter().any(|m| line_lower.contains(m)) {
            let column = line_text.find("Vec::with_capacity(").unwrap_or(0) as u32;
            return Some(MisuseFinding {
                file: file.to_string(),
                line,
                column,
                rule: rule.clone(),
                api_call: "Vec::with_capacity".to_string(),
                message: "Vec::with_capacity appears to use unbounded external input".to_string(),
                fix_suggestion:
                    "Clamp requested capacity with a hard upper bound before allocation".to_string(),
                code_context: line_text.to_string(),
            });
        }
    }
    None
}

/// Check for detached tokio tasks.
fn check_detached_tokio_spawn(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
) -> Option<MisuseFinding> {
    if line_text.contains("tokio::spawn(")
        && !line_text.contains('=')
        && !line_text.contains("handles.push")
    {
        let column = line_text.find("tokio::spawn(").unwrap_or(0) as u32;
        return Some(MisuseFinding {
            file: file.to_string(),
            line,
            column,
            rule: rule.clone(),
            api_call: "tokio::spawn".to_string(),
            message: "tokio::spawn used without keeping JoinHandle".to_string(),
            fix_suggestion: "Store JoinHandle values and await them to surface task errors"
                .to_string(),
            code_context: line_text.to_string(),
        });
    }
    None
}

/// Check for map iteration order assumptions.
fn check_hashmap_order_dependence(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
    rust_ctx: &RustLineContext<'_>,
) -> Option<MisuseFinding> {
    let looks_like_hashmap_iteration = line_text.contains(".iter()")
        && (line_text.contains("for ") || rust_ctx.previous_line.starts_with("for "))
        && rust_ctx.file_has_hashmap;
    if looks_like_hashmap_iteration {
        let column = line_text.find(".iter()").unwrap_or(0) as u32;
        return Some(MisuseFinding {
            file: file.to_string(),
            line,
            column,
            rule: rule.clone(),
            api_call: "HashMap::iter".to_string(),
            message: "Potential logic dependence on HashMap iteration order".to_string(),
            fix_suggestion: "Use BTreeMap/IndexMap or sort keys before ordered operations"
                .to_string(),
            code_context: line_text.to_string(),
        });
    }
    None
}

/// Check for clone usage in loop bodies.
fn check_clone_in_hot_loop(
    rule: &APIRule,
    file: &str,
    line: u32,
    line_text: &str,
    rust_ctx: &RustLineContext<'_>,
) -> Option<MisuseFinding> {
    if line_text.contains(".clone()")
        && (line_text.contains("for ") || line_text.contains("while ") || rust_ctx.previous_is_loop)
    {
        let column = line_text.find(".clone()").unwrap_or(0) as u32;
        return Some(MisuseFinding {
            file: file.to_string(),
            line,
            column,
            rule: rule.clone(),
            api_call: "clone".to_string(),
            message: "clone() in loop context may create avoidable allocation overhead".to_string(),
            fix_suggestion: "Prefer borrowing/references or move semantics inside hot loops"
                .to_string(),
            code_context: line_text.to_string(),
        });
    }
    None
}

// =============================================================================
// Filtering
// =============================================================================

/// Filter findings by category and severity
fn filter_findings(
    findings: Vec<MisuseFinding>,
    categories: Option<&[MisuseCategory]>,
    severities: Option<&[MisuseSeverity]>,
) -> Vec<MisuseFinding> {
    findings
        .into_iter()
        .filter(|f| {
            // Category filter
            if let Some(cats) = categories {
                if !cats.contains(&f.rule.category) {
                    return false;
                }
            }

            // Severity filter
            if let Some(sevs) = severities {
                if !sevs.contains(&f.rule.severity) {
                    return false;
                }
            }

            true
        })
        .collect()
}

// =============================================================================
// Summary Building
// =============================================================================

/// Build summary from findings
fn build_summary(findings: &[MisuseFinding], files_scanned: u32) -> APICheckSummary {
    let mut by_category: HashMap<String, u32> = HashMap::new();
    let mut by_severity: HashMap<String, u32> = HashMap::new();
    let mut apis_checked: Vec<String> = Vec::new();

    for finding in findings {
        // Count by category
        let cat_str = format!("{:?}", finding.rule.category).to_lowercase();
        *by_category.entry(cat_str).or_insert(0) += 1;

        // Count by severity
        let sev_str = format!("{:?}", finding.rule.severity).to_lowercase();
        *by_severity.entry(sev_str).or_insert(0) += 1;

        // Track APIs
        if !apis_checked.contains(&finding.api_call) {
            apis_checked.push(finding.api_call.clone());
        }
    }

    APICheckSummary {
        total_findings: findings.len() as u32,
        by_category,
        by_severity,
        apis_checked,
        files_scanned,
    }
}

// =============================================================================
// Output Formatting
// =============================================================================

/// Format report as human-readable text
fn format_api_check_text(report: &APICheckReport) -> String {
    let mut output = String::new();

    output.push_str("=== API Check Report ===\n\n");

    // Summary
    output.push_str(&format!(
        "Files scanned: {}\n",
        report.summary.files_scanned
    ));
    output.push_str(&format!("Rules applied: {}\n", report.rules_applied));
    output.push_str(&format!(
        "Total findings: {}\n\n",
        report.summary.total_findings
    ));

    // By severity
    if !report.summary.by_severity.is_empty() {
        output.push_str("By Severity:\n");
        for (severity, count) in &report.summary.by_severity {
            output.push_str(&format!("  {}: {}\n", severity, count));
        }
        output.push('\n');
    }

    // By category
    if !report.summary.by_category.is_empty() {
        output.push_str("By Category:\n");
        for (category, count) in &report.summary.by_category {
            output.push_str(&format!("  {}: {}\n", category, count));
        }
        output.push('\n');
    }

    // Findings
    if !report.findings.is_empty() {
        output.push_str("Findings:\n");
        output.push_str(&"-".repeat(60));
        output.push('\n');

        for finding in &report.findings {
            output.push_str(&format!(
                "[{:?}] {} ({})\n",
                finding.rule.severity, finding.rule.name, finding.rule.id
            ));
            output.push_str(&format!(
                "  Location: {}:{}:{}\n",
                finding.file, finding.line, finding.column
            ));
            output.push_str(&format!("  API: {}\n", finding.api_call));
            output.push_str(&format!("  Message: {}\n", finding.message));
            output.push_str(&format!("  Fix: {}\n", finding.fix_suggestion));
            if !finding.code_context.is_empty() {
                output.push_str(&format!("  Context: {}\n", finding.code_context.trim()));
            }
            output.push('\n');
        }
    } else {
        output.push_str("No API misuse patterns detected.\n");
    }

    output
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_python_rules_defined() {
        let rules = python_rules();
        assert!(!rules.is_empty());
        assert!(rules.iter().any(|r| r.id == "PY001")); // missing-timeout
        assert!(rules.iter().any(|r| r.id == "PY002")); // bare-except
        assert!(rules.iter().any(|r| r.id == "PY003")); // weak-hash-md5
        assert!(rules.iter().any(|r| r.id == "PY005")); // unclosed-file
    }

    #[test]
    fn test_rust_rules_defined() {
        let rules = rust_rules();
        assert!(!rules.is_empty());
        assert!(rules.iter().any(|r| r.id == "RS001"));
        assert!(rules.iter().any(|r| r.id == "RS002"));
        assert!(rules.iter().any(|r| r.id == "RS003"));
        assert!(rules.iter().any(|r| r.id == "RS004"));
        assert!(rules.iter().any(|r| r.id == "RS005"));
        assert!(rules.iter().any(|r| r.id == "RS006"));
    }

    #[test]
    fn test_all_supported_languages_have_rules() {
        for language in all_api_languages() {
            let rules = rules_for_language(*language);
            assert!(
                !rules.is_empty(),
                "expected at least one api-check rule for {:?}",
                language
            );
        }
    }

    #[test]
    fn test_detect_language_extended_extensions() {
        let cases = [
            ("main.go", ApiLanguage::Go),
            ("Main.java", ApiLanguage::Java),
            ("app.js", ApiLanguage::JavaScript),
            ("component.tsx", ApiLanguage::TypeScript),
            ("main.c", ApiLanguage::C),
            ("main.cpp", ApiLanguage::Cpp),
            ("app.rb", ApiLanguage::Ruby),
            ("index.php", ApiLanguage::Php),
            ("Main.kt", ApiLanguage::Kotlin),
            ("main.swift", ApiLanguage::Swift),
            ("Program.cs", ApiLanguage::CSharp),
            ("Main.scala", ApiLanguage::Scala),
            ("app.ex", ApiLanguage::Elixir),
            ("main.lua", ApiLanguage::Lua),
            ("game.luau", ApiLanguage::Luau),
            ("main.ml", ApiLanguage::Ocaml),
        ];

        for (path, expected) in cases {
            assert_eq!(detect_language(Path::new(path)), Some(expected), "{path}");
        }
    }

    #[test]
    fn test_check_missing_timeout() {
        let rule = &python_rules()[0]; // PY001

        // Should detect
        let finding = check_missing_timeout(rule, "test.py", 1, "response = requests.get(url)");
        assert!(finding.is_some());

        // Should not detect (has timeout)
        let finding = check_missing_timeout(
            rule,
            "test.py",
            1,
            "response = requests.get(url, timeout=30)",
        );
        assert!(finding.is_none());
    }

    #[test]
    fn test_check_bare_except() {
        let rule = &python_rules()[1]; // PY002

        // Should detect
        let finding = check_bare_except(rule, "test.py", 1, "except:");
        assert!(finding.is_some());

        // Should not detect (has exception type)
        let finding = check_bare_except(rule, "test.py", 1, "except Exception:");
        assert!(finding.is_none());
    }

    #[test]
    fn test_check_md5_usage() {
        let rule = &python_rules()[2]; // PY003

        // Should detect
        let finding = check_md5_usage(rule, "test.py", 1, "hash = hashlib.md5(data)");
        assert!(finding.is_some());

        // Should not detect
        let finding = check_md5_usage(rule, "test.py", 1, "hash = hashlib.sha256(data)");
        assert!(finding.is_none());
    }

    #[test]
    fn test_check_unclosed_file() {
        let rule = &python_rules()[4]; // PY005

        // Should detect
        let finding = check_unclosed_file(rule, "test.py", 1, "f = open('data.txt')");
        assert!(finding.is_some());

        // Should not detect (using context manager)
        let finding = check_unclosed_file(rule, "test.py", 1, "with open('data.txt') as f:");
        assert!(finding.is_none());
    }

    #[test]
    fn test_filter_by_category() {
        let findings = vec![
            MisuseFinding {
                file: "test.py".to_string(),
                line: 1,
                column: 0,
                rule: APIRule {
                    id: "PY001".to_string(),
                    name: "test".to_string(),
                    category: MisuseCategory::Parameters,
                    severity: MisuseSeverity::High,
                    description: "test".to_string(),
                    correct_usage: "test".to_string(),
                },
                api_call: "test".to_string(),
                message: "test".to_string(),
                fix_suggestion: "test".to_string(),
                code_context: "test".to_string(),
            },
            MisuseFinding {
                file: "test.py".to_string(),
                line: 2,
                column: 0,
                rule: APIRule {
                    id: "PY003".to_string(),
                    name: "test".to_string(),
                    category: MisuseCategory::Crypto,
                    severity: MisuseSeverity::High,
                    description: "test".to_string(),
                    correct_usage: "test".to_string(),
                },
                api_call: "test".to_string(),
                message: "test".to_string(),
                fix_suggestion: "test".to_string(),
                code_context: "test".to_string(),
            },
        ];

        let filtered = filter_findings(findings, Some(&[MisuseCategory::Crypto]), None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].rule.category, MisuseCategory::Crypto);
    }

    #[test]
    fn test_build_summary() {
        let findings = vec![MisuseFinding {
            file: "test.py".to_string(),
            line: 1,
            column: 0,
            rule: APIRule {
                id: "PY001".to_string(),
                name: "test".to_string(),
                category: MisuseCategory::Parameters,
                severity: MisuseSeverity::High,
                description: "test".to_string(),
                correct_usage: "test".to_string(),
            },
            api_call: "requests.get".to_string(),
            message: "test".to_string(),
            fix_suggestion: "test".to_string(),
            code_context: "test".to_string(),
        }];

        let summary = build_summary(&findings, 5);
        assert_eq!(summary.total_findings, 1);
        assert_eq!(summary.files_scanned, 5);
        assert!(summary.apis_checked.contains(&"requests.get".to_string()));
    }

    #[test]
    fn test_collect_files_includes_rust() {
        let temp = TempDir::new().unwrap();
        let py = temp.path().join("a.py");
        let rs = temp.path().join("b.rs");
        let go = temp.path().join("c.go");
        let txt = temp.path().join("c.txt");
        fs::write(&py, "print('ok')").unwrap();
        fs::write(&rs, "fn main() {}").unwrap();
        fs::write(&go, "package main").unwrap();
        fs::write(&txt, "ignore").unwrap();

        let files = collect_files(temp.path()).unwrap();
        assert!(files.iter().any(|f| f.ends_with("a.py")));
        assert!(files.iter().any(|f| f.ends_with("b.rs")));
        assert!(files.iter().any(|f| f.ends_with("c.go")));
        assert!(!files.iter().any(|f| f.ends_with("c.txt")));
    }

    #[test]
    fn test_check_mutex_lock_unwrap() {
        let rule = &rust_rules()[0];
        let finding =
            check_mutex_lock_unwrap(rule, "lib.rs", 10, "let guard = shared.lock().unwrap();");
        assert!(finding.is_some());
    }

    #[test]
    fn test_check_file_open_without_context() {
        let rule = &rust_rules()[1];
        let finding = check_file_open_without_context(rule, "lib.rs", 8, "let f = File::open(p)?;");
        assert!(finding.is_some());

        let contextual = check_file_open_without_context(
            rule,
            "lib.rs",
            9,
            "let f = File::open(p).with_context(|| \"open\".to_string())?;",
        );
        assert!(contextual.is_none());
    }

    #[test]
    fn test_check_unbounded_with_capacity() {
        let rule = &rust_rules()[2];
        let finding =
            check_unbounded_with_capacity(rule, "lib.rs", 12, "let v = Vec::with_capacity(len);");
        assert!(finding.is_some());

        let bounded = check_unbounded_with_capacity(
            rule,
            "lib.rs",
            13,
            "let v = Vec::with_capacity(256);",
        );
        assert!(bounded.is_none());
    }

    #[test]
    fn test_check_tokio_spawn_detached() {
        let rule = &rust_rules()[3];
        let detached = check_detached_tokio_spawn(
            rule,
            "lib.rs",
            3,
            "tokio::spawn(async move { work().await; });",
        );
        let tracked = check_detached_tokio_spawn(
            rule,
            "lib.rs",
            4,
            "let handle = tokio::spawn(async move { work().await; });",
        );
        assert!(detached.is_some());
        assert!(tracked.is_none());
    }

    #[test]
    fn test_check_hashmap_order_dependence() {
        let rule = &rust_rules()[4];
        let ctx = RustLineContext {
            file_has_hashmap: true,
            previous_line: "for (k, v) in map",
            previous_is_loop: true,
        };
        let finding = check_hashmap_order_dependence(rule, "lib.rs", 12, "    .iter()", &ctx);
        assert!(finding.is_some());
    }

    #[test]
    fn test_check_clone_in_hot_loop() {
        let rule = &rust_rules()[5];
        let ctx = RustLineContext {
            file_has_hashmap: false,
            previous_line: "for item in items {",
            previous_is_loop: true,
        };
        let finding = check_clone_in_hot_loop(rule, "lib.rs", 20, "value.clone()", &ctx);
        assert!(finding.is_some());
    }

    fn assert_language_findings(
        filename: &str,
        language: ApiLanguage,
        source: &str,
        expected_rule_id: &str,
    ) {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join(filename);
        fs::write(&path, source).unwrap();
        let rules = rules_for_language(language);
        let findings = analyze_file(&path, &rules, language).unwrap();
        assert!(
            findings.iter().any(|finding| finding.rule.id == expected_rule_id),
            "expected {expected_rule_id} for {filename}, got {:?}",
            findings.iter().map(|f| f.rule.id.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extended_language_rule_detection() {
        let cases = [
            ("main.go", ApiLanguage::Go, "data, _ := ioutil.ReadFile(path)", "GO001"),
            (
                "Main.java",
                ApiLanguage::Java,
                "if (name == otherName) { }",
                "JV001",
            ),
            ("app.js", ApiLanguage::JavaScript, "if (a == b) {}", "JS001"),
            ("app.ts", ApiLanguage::TypeScript, "if (a == b) {}", "TS001"),
            ("main.c", ApiLanguage::C, "gets(buffer);", "C001"),
            ("main.cpp", ApiLanguage::Cpp, "std::auto_ptr<Foo> p;", "CPP003"),
            ("app.rb", ApiLanguage::Ruby, "eval(params[:code])", "RB001"),
            ("index.php", ApiLanguage::Php, "unserialize($payload);", "PH005"),
            ("Main.kt", ApiLanguage::Kotlin, "val name = user!!", "KT001"),
            ("main.swift", ApiLanguage::Swift, "let name = value!", "SW003"),
            ("Program.cs", ApiLanguage::CSharp, "var x = task.Result;", "CS003"),
            ("Main.scala", ApiLanguage::Scala, "val casted = value.asInstanceOf[String]", "SC002"),
            ("app.ex", ApiLanguage::Elixir, "String.to_atom(param)", "EX001"),
            ("main.lua", ApiLanguage::Lua, "value = 1", "LU001"),
            ("game.luau", ApiLanguage::Luau, "os.execute(cmd)", "LU003"),
            ("main.ml", ApiLanguage::Ocaml, "Obj.magic value", "OC004"),
        ];

        for (filename, language, source, expected_rule_id) in cases {
            assert_language_findings(filename, language, source, expected_rule_id);
        }
    }
}
