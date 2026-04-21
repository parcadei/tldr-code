# Remaining Commands - Rust Port Specification

**Created:** 2026-02-04
**Author:** architect-agent
**Source:** Python v1 at `/Users/cosimo/.opc-dev/opc/packages/tldr-code/tldr/cli/commands/`
**Target:** `/Users/cosimo/.opc-dev/opc/packages/tldr-code/tldr-rs/crates/tldr-cli/src/commands/remaining/`

## Overview

This specification defines the Rust port of 9 TLDR commands for code analysis, security scanning, and developer productivity. Commands are grouped by complexity for phased implementation.

## Complexity Groupings

| Complexity | Commands | Notes |
|------------|----------|-------|
| **LOW** | `todo`, `explain`, `secure` | Thin wrappers delegating to sub-analyses |
| **MEDIUM** | `definition`, `diff`, `diff-impact` | AST-based with moderate logic |
| **HIGH** | `api-check`, `equivalence`, `vuln` | Complex pattern matching, taint analysis |

---

## Module Architecture

```
remaining/
├── mod.rs              # Module exports and re-exports
├── types.rs            # Shared data types across all commands
├── error.rs            # RemainingError enum and Result type
├── validation.rs       # Path safety, resource limits (TIGER mitigations)
│
├── todo.rs             # todo command - improvement suggestions
├── explain.rs          # explain command - composite function analysis
├── secure.rs           # secure command - security dashboard
│
├── definition.rs       # definition command - go-to-definition
├── references.rs       # references command - find all references
├── diff.rs             # diff command - AST-aware structural diff
├── diff_impact.rs      # diff-impact command - change impact analysis
│
├── api_check.rs        # api-check command - API misuse detection
├── equivalence.rs      # equivalence command - GVN redundancy
└── vuln.rs             # vuln command - vulnerability detection
```

---

## Shared Types (`types.rs`)

### Output Format

```rust
use serde::{Deserialize, Serialize};
use clap::ValueEnum;

/// Output format for all commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
    Text,
    Sarif, // Only for vuln command
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Json
    }
}
```

### Severity Level

```rust
/// Severity level for findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl Severity {
    pub fn order(&self) -> u8 {
        match self {
            Self::Critical => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
            Self::Info => 4,
        }
    }
}
```

### Location

```rust
/// A location in source code (shared across multiple commands).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub line: u32,
    #[serde(default)]
    pub column: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_column: Option<u32>,
}
```

---

## Error Types (`error.rs`)

```rust
use std::path::PathBuf;
use thiserror::Error;

/// Errors for remaining commands.
#[derive(Debug, Error)]
pub enum RemainingError {
    /// File not found.
    #[error("file not found: {}", path.display())]
    FileNotFound { path: PathBuf },

    /// Function/symbol not found.
    #[error("symbol '{symbol}' not found in {}", file.display())]
    SymbolNotFound { symbol: String, file: PathBuf },

    /// Parse error.
    #[error("parse error in {}: {message}", file.display())]
    ParseError { file: PathBuf, message: String },

    /// Invalid arguments.
    #[error("invalid argument: {message}")]
    InvalidArgument { message: String },

    /// File too large.
    #[error("file too large: {} ({bytes} bytes)", path.display())]
    FileTooLarge { path: PathBuf, bytes: u64 },

    /// Path traversal blocked.
    #[error("path traversal blocked: {}", path.display())]
    PathTraversal { path: PathBuf },

    /// Unsupported language.
    #[error("unsupported language: {language}")]
    UnsupportedLanguage { language: String },

    /// Analysis error.
    #[error("analysis error: {message}")]
    AnalysisError { message: String },

    /// Findings detected (for vuln/api-check - special exit code).
    #[error("{count} findings detected")]
    FindingsDetected { count: u32 },

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type RemainingResult<T> = Result<T, RemainingError>;
```

---

# LOW Complexity Commands

---

## 1. todo

**Purpose:** Aggregate improvement suggestions from multiple analyses, priority-sorted.

**Python Source:** `todo_cmd.py` + `todo.py`

### CLI Interface

```
tldr todo <path> [OPTIONS]

Arguments:
  <path>  File or directory to analyze (required)

Options:
  -f, --format <FORMAT>  Output format [default: json] [possible values: json, text]
  --detail <ANALYSIS>    Show details for specific sub-analysis
  --quick                Run quick mode (skip similar analysis)
  --lang <LANG>          Language hint [default: auto]
```

### Args Struct

```rust
#[derive(Debug, Args)]
pub struct TodoArgs {
    /// File or directory to analyze
    pub path: PathBuf,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,

    /// Show details for specific sub-analysis
    #[arg(long)]
    pub detail: Option<String>,

    /// Run quick mode (skip similar analysis)
    #[arg(long)]
    pub quick: bool,

    /// Language hint
    #[arg(long, default_value = "auto")]
    pub lang: String,
}
```

### Output Types

```rust
/// A single improvement item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub category: String,
    pub priority: u32,
    pub description: String,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub line: u32,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub score: f64,
}

/// Summary of todo analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoSummary {
    pub dead_count: u32,
    pub similar_pairs: u32,
    pub low_cohesion_count: u32,
    pub hotspot_count: u32,
    pub equivalence_groups: u32,
}

/// Todo analysis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoReport {
    pub wrapper: String,
    pub path: String,
    pub items: Vec<TodoItem>,
    pub summary: TodoSummary,
    pub sub_results: HashMap<String, serde_json::Value>,
    pub total_elapsed_ms: f64,
}
```

### Behavioral Contract

1. Runs sub-analyses: `dead`, `complexity`, `cohesion`, `equivalence`, (optional) `similar`
2. Aggregates findings into priority-sorted `TodoItem` list
3. Supports `--detail <name>` to show raw sub-analysis results
4. `--quick` skips `similar` (slowest analysis)

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (file not found, parse error) |

### Dependencies

- `dead` analysis module
- `complexity` analysis module
- `cohesion` analysis module
- `equivalence` (GVN) module
- `similar` module (optional)

### Complexity Estimate: **LOW**

Thin wrapper that orchestrates existing analyses. Main logic is aggregation and sorting.

---

## 2. explain

**Purpose:** Composite function analysis combining signature, purity, complexity, and call relationships.

**Python Source:** `explain_cmd.py` + `explain.py`

### CLI Interface

```
tldr explain <file> <function> [OPTIONS]

Arguments:
  <file>      Source file to analyze (required)
  <function>  Function name to explain (required)

Options:
  -f, --format <FORMAT>  Output format [default: json] [possible values: json, text]
  --depth <N>            Call graph depth [default: 2]
  --lang <LANG>          Language hint [default: auto]
```

### Args Struct

```rust
#[derive(Debug, Args)]
pub struct ExplainArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function name to explain
    pub function: String,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,

    /// Call graph depth
    #[arg(long, default_value = "2")]
    pub depth: u32,

    /// Language hint
    #[arg(long, default_value = "auto")]
    pub lang: String,
}
```

### Output Types

```rust
/// Parameter information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamInfo {
    pub name: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_hint: Option<String>,
}

/// Function signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureInfo {
    pub params: Vec<ParamInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    #[serde(default)]
    pub decorators: Vec<String>,
    #[serde(default)]
    pub is_async: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
}

/// Purity analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurityInfo {
    pub classification: String, // "pure", "impure", "unknown"
    #[serde(default)]
    pub effects: Vec<String>,
    pub confidence: String, // "high", "medium", "low"
}

/// Complexity metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityInfo {
    pub cyclomatic: u32,
    pub num_blocks: u32,
    pub num_edges: u32,
    pub has_loops: bool,
}

/// Caller/callee info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallInfo {
    pub name: String,
    pub file: String,
    pub line: u32,
}

/// Full explain report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainReport {
    pub function_name: String,
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    pub language: String,
    pub signature: SignatureInfo,
    pub purity: PurityInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity: Option<ComplexityInfo>,
    #[serde(default)]
    pub callers: Vec<CallInfo>,
    #[serde(default)]
    pub callees: Vec<CallInfo>,
}
```

### Behavioral Contract

1. Finds function in file by name
2. Extracts signature (params, return type, decorators, docstring)
3. Runs purity analysis
4. Computes complexity metrics (cyclomatic, blocks, edges)
5. Finds callers and callees up to `--depth`

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | File/function not found or error |

### Dependencies

- AST extractor
- Purity analyzer
- CFG builder (for complexity)
- Call graph module

### Complexity Estimate: **LOW**

Combines existing analyses into a single report. No new analysis logic.

---

## 3. secure

**Purpose:** Security analysis dashboard aggregating multiple security checks.

**Python Source:** `secure_cmd.py` + `secure.py`

### CLI Interface

```
tldr secure <path> [OPTIONS]

Arguments:
  <path>  File or directory to analyze (required)

Options:
  -f, --format <FORMAT>  Output format [default: json] [possible values: json, text]
  --detail <ANALYSIS>    Show details for specific sub-analysis
  --quick                Run quick mode (taint, resources, bounds only)
  --lang <LANG>          Language hint [default: auto]
```

### Args Struct

```rust
#[derive(Debug, Args)]
pub struct SecureArgs {
    /// File or directory to analyze
    pub path: PathBuf,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,

    /// Show details for specific sub-analysis
    #[arg(long)]
    pub detail: Option<String>,

    /// Run quick mode
    #[arg(long)]
    pub quick: bool,

    /// Language hint
    #[arg(long, default_value = "auto")]
    pub lang: String,
}
```

### Output Types

```rust
/// A security finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureFinding {
    pub category: String,
    pub severity: String,
    pub description: String,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub line: u32,
}

/// Security summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureSummary {
    pub taint_count: u32,
    pub taint_critical: u32,
    pub leak_count: u32,
    pub bounds_warnings: u32,
    pub missing_contracts: u32,
    pub mutable_params: u32,
}

/// Secure analysis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureReport {
    pub wrapper: String,
    pub path: String,
    pub findings: Vec<SecureFinding>,
    pub summary: SecureSummary,
    pub sub_results: HashMap<String, serde_json::Value>,
    pub total_elapsed_ms: f64,
}
```

### Behavioral Contract

1. Quick mode runs: `taint`, `resources`, `bounds`
2. Full mode adds: `contracts`, `behavioral`, `annotated`, `mutability`
3. Aggregates findings sorted by severity (critical first)
4. Supports `--detail <name>` to show raw sub-analysis

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error |

### Dependencies

- Taint analyzer
- Resource leak analyzer
- Bounds checker
- Contract analyzer
- Behavioral analyzer
- Mutability analyzer

### Complexity Estimate: **LOW**

Orchestration wrapper. All heavy lifting done by sub-analyses.

---

# MEDIUM Complexity Commands

---

## 4. definition

**Purpose:** Go-to-definition - find where a symbol is defined.

**Python Source:** `definition_cmd.py`

### CLI Interface

```
tldr definition <file> <line> <column> [OPTIONS]
tldr definition --symbol NAME --file FILE [OPTIONS]

Arguments:
  <file>    Source file (positional mode)
  <line>    Line number (1-indexed)
  <column>  Column number (0-indexed)

Options:
  --symbol <NAME>        Find symbol by name instead of position
  --file <FILE>          File to search in (with --symbol)
  --project <PATH>       Project root for cross-file search
  --lang <LANG>          Language hint [default: auto]
  -f, --format <FORMAT>  Output format [default: json]
```

### Args Struct

```rust
#[derive(Debug, Args)]
pub struct DefinitionArgs {
    /// Source file (positional mode)
    #[arg(required_unless_present = "symbol")]
    pub file_pos: Option<PathBuf>,

    /// Line number (1-indexed)
    #[arg(required_unless_present = "symbol")]
    pub line: Option<u32>,

    /// Column number (0-indexed)
    #[arg(required_unless_present = "symbol")]
    pub column: Option<u32>,

    /// Find symbol by name
    #[arg(long)]
    pub symbol: Option<String>,

    /// File to search in (with --symbol)
    #[arg(long)]
    pub file: Option<PathBuf>,

    /// Project root for cross-file search
    #[arg(long)]
    pub project: Option<PathBuf>,

    /// Language hint
    #[arg(long, default_value = "auto")]
    pub lang: String,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

### Output Types

```rust
/// Symbol kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Variable,
    Parameter,
    Constant,
    Module,
    Type,
    Interface,
    Property,
    Unknown,
}

/// Symbol information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_annotation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    #[serde(default)]
    pub is_builtin: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

/// Definition result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefinitionResult {
    pub symbol: SymbolInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_definition: Option<Location>,
}
```

### Behavioral Contract

1. Positional mode: Find symbol at (file, line, column), then find its definition
2. Name mode: Find definition of named symbol in file/project
3. Cross-file resolution via import tracking
4. Reports `is_builtin: true` for language builtins

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (file not found, symbol not found) |

### Dependencies

- Tree-sitter parsers (17 languages)
- Import resolver
- AST extractor

### Complexity Estimate: **MEDIUM**

Cross-file resolution requires import tracking. Position-to-symbol mapping uses tree-sitter.

---

## 5. references

**Purpose:** Find all references to a symbol.

**Python Source:** `definition_cmd.py` (cmd_references)

### CLI Interface

```
tldr references <file> <line> <column> [OPTIONS]

Arguments:
  <file>    Source file
  <line>    Line number (1-indexed)
  <column>  Column number (0-indexed)

Options:
  --project <PATH>       Project root for cross-file search
  --lang <LANG>          Language hint [default: auto]
  -f, --format <FORMAT>  Output format [default: json]
```

### Args Struct

```rust
#[derive(Debug, Args)]
pub struct ReferencesArgs {
    /// Source file
    pub file: PathBuf,

    /// Line number (1-indexed)
    pub line: u32,

    /// Column number (0-indexed)
    pub column: u32,

    /// Project root for cross-file search
    #[arg(long)]
    pub project: Option<PathBuf>,

    /// Language hint
    #[arg(long, default_value = "auto")]
    pub lang: String,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

### Output Types

```rust
/// A reference to a symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub location: Location,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub is_definition: bool,
    #[serde(default)]
    pub is_write: bool,
}

/// References result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferencesResult {
    pub symbol: SymbolInfo,
    pub references: Vec<Reference>,
    pub total_count: u32,
}
```

### Behavioral Contract

1. Find symbol at position
2. Search project for all uses of that symbol
3. Classify each reference as definition/write/read
4. Include context (surrounding line text)

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error |

### Dependencies

- Same as `definition`
- Project-wide file scanning

### Complexity Estimate: **MEDIUM**

---

## 6. diff

**Purpose:** AST-aware structural diff between two files.

**Python Source:** `diff_cmd.py`

### CLI Interface

```
tldr diff <file_a> <file_b> [OPTIONS]

Arguments:
  <file_a>  First file to compare (required)
  <file_b>  Second file to compare (required)

Options:
  --semantic-only        Exclude formatting-only changes
  --detect-moves         Detect moved code blocks [default: true]
  --lang <LANG>          Language hint [default: auto]
  -f, --format <FORMAT>  Output format [default: json]
```

### Args Struct

```rust
#[derive(Debug, Args)]
pub struct DiffArgs {
    /// First file to compare
    pub file_a: PathBuf,

    /// Second file to compare
    pub file_b: PathBuf,

    /// Exclude formatting-only changes
    #[arg(long)]
    pub semantic_only: bool,

    /// Detect moved code blocks
    #[arg(long, default_value = "true")]
    pub detect_moves: bool,

    /// Language hint
    #[arg(long, default_value = "auto")]
    pub lang: String,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

### Output Types

```rust
/// Type of AST change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Insert,
    Delete,
    Update,
    Move,
    Rename,
    Extract,
    Inline,
    Format,
}

/// Kind of AST node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Function,
    Class,
    Method,
    Statement,
    Expression,
    Block,
}

/// A single AST change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ASTChange {
    pub change_type: ChangeType,
    pub node_kind: NodeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_location: Option<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_location: Option<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f64>,
}

/// Diff summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffSummary {
    pub total_changes: u32,
    pub semantic_changes: u32,
    pub inserts: u32,
    pub deletes: u32,
    pub updates: u32,
    pub moves: u32,
    pub renames: u32,
    pub formats: u32,
    pub extracts: u32,
}

/// Diff report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffReport {
    pub file_a: String,
    pub file_b: String,
    pub identical: bool,
    pub changes: Vec<ASTChange>,
    pub summary: DiffSummary,
}
```

### Behavioral Contract

1. Parse both files to AST
2. Extract function/class nodes
3. Match nodes between versions by name
4. Detect:
   - Inserts (new functions/classes)
   - Deletes (removed)
   - Updates (modified body)
   - Moves (same content, different location)
   - Renames (similar content, different name)
   - Format (whitespace only)
5. `--semantic-only` filters out Format changes

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error |

### Dependencies

- AST extractor
- AST normalizer (for structural comparison)
- Similarity computation (difflib-like)

### Complexity Estimate: **MEDIUM**

Move detection and similarity scoring add moderate complexity.

---

## 7. diff-impact

**Purpose:** Analyze impact of code changes - identify affected functions and suggest tests.

**Python Source:** `diff_impact_cmd.py` + `diff_impact.py`

### CLI Interface

```
tldr diff-impact [OPTIONS]

Options:
  --files <FILES>...     Explicit list of changed files
  --git                  Get changed files from git
  --git-base <REF>       Git ref for diff base [default: HEAD~1]
  --depth <N>            Caller search depth [default: 3]
  -f, --format <FORMAT>  Output format [default: json]
```

### Args Struct

```rust
#[derive(Debug, Args)]
pub struct DiffImpactArgs {
    /// Explicit list of changed files
    #[arg(long, num_args = 1..)]
    pub files: Option<Vec<PathBuf>>,

    /// Get changed files from git
    #[arg(long)]
    pub git: bool,

    /// Git ref for diff base
    #[arg(long, default_value = "HEAD~1")]
    pub git_base: String,

    /// Caller search depth
    #[arg(long, default_value = "3")]
    pub depth: u32,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

### Output Types

```rust
/// A function affected by changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFunction {
    pub name: String,
    pub file: String,
    pub line: u32,
    #[serde(default)]
    pub callers: Vec<CallInfo>,
}

/// Diff impact summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffImpactSummary {
    pub files_changed: u32,
    pub functions_changed: u32,
    pub tests_to_run: u32,
}

/// Diff impact report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffImpactReport {
    pub changed_functions: Vec<ChangedFunction>,
    pub suggested_tests: Vec<String>,
    pub summary: DiffImpactSummary,
}
```

### Behavioral Contract

1. Get changed files (from `--files` or `git diff`)
2. Identify functions in changed files
3. Find transitive callers up to `--depth`
4. Suggest test files that might test affected functions

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error |

### Dependencies

- Git integration
- AST extractor
- Call graph module

### Complexity Estimate: **MEDIUM**

Git integration and transitive caller resolution.

---

# HIGH Complexity Commands

---

## 8. api-check

**Purpose:** Detect API misuse patterns (unclosed resources, missing timeouts, insecure practices).

**Python Source:** `api_check_cmd.py`

### CLI Interface

```
tldr api-check <path> [OPTIONS]

Arguments:
  <path>  File or directory to analyze (required)

Options:
  --category <CAT>       Filter by category [values: call_order, error_handling, parameters, resources, crypto, concurrency, security]
  --severity <SEV>       Filter by severity [values: info, low, medium, high]
  --lang <LANG>          Language hint [default: auto]
  -f, --format <FORMAT>  Output format [default: json]
```

### Args Struct

```rust
#[derive(Debug, Args)]
pub struct ApiCheckArgs {
    /// File or directory to analyze
    pub path: PathBuf,

    /// Filter by category
    #[arg(long)]
    pub category: Option<MisuseCategory>,

    /// Filter by severity
    #[arg(long)]
    pub severity: Option<MisuseSeverity>,

    /// Language hint
    #[arg(long, default_value = "auto")]
    pub lang: String,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

### Output Types

```rust
/// Categories of API misuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum MisuseCategory {
    CallOrder,
    ErrorHandling,
    Parameters,
    Resources,
    Crypto,
    Concurrency,
    Security,
}

/// Severity of API misuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum MisuseSeverity {
    Info,
    Low,
    Medium,
    High,
}

/// An API rule definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct APIRule {
    pub id: String,
    pub name: String,
    pub category: MisuseCategory,
    pub severity: MisuseSeverity,
    pub description: String,
    pub correct_usage: String,
}

/// A detected API misuse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MisuseFinding {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub rule: APIRule,
    pub api_call: String,
    pub message: String,
    pub fix_suggestion: String,
    #[serde(default)]
    pub code_context: String,
}

/// API check summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct APICheckSummary {
    pub total_findings: u32,
    pub by_category: HashMap<String, u32>,
    pub by_severity: HashMap<String, u32>,
    pub apis_checked: Vec<String>,
    pub files_scanned: u32,
}

/// API check report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct APICheckReport {
    pub findings: Vec<MisuseFinding>,
    pub summary: APICheckSummary,
    pub rules_applied: u32,
}
```

### Behavioral Contract

Detects patterns including:

| Category | Examples |
|----------|----------|
| Resources | File not closed, cursor not closed |
| Parameters | `requests.get` without timeout |
| Security | `pickle.loads`, `eval()` |
| Crypto | MD5/SHA1 for passwords, `random.random()` for crypto |
| Error Handling | Bare `except:`, promise without catch |
| Concurrency | Race conditions |
| Call Order | API protocol violations |

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success, no findings |
| 1 | Error |
| 2 | Findings detected (if specified) |

### Dependencies

- AST extractor
- Pattern matcher (rule engine)
- Multi-language support (Python, TypeScript)

### Complexity Estimate: **HIGH**

Extensive rule database, multiple language support, context-sensitive pattern matching.

---

## 9. equivalence

**Purpose:** GVN (Global Value Numbering) to detect redundant expressions.

**Python Source:** `equivalence_cmd.py` + `dfg/gvn.py`

### CLI Interface

```
tldr equivalence <file> [function] [OPTIONS]

Arguments:
  <file>      Source file to analyze (required)
  [function]  Specific function (default: all)

Options:
  -f, --format <FORMAT>  Output format [default: json]
```

### Args Struct

```rust
#[derive(Debug, Args)]
pub struct EquivalenceArgs {
    /// Source file to analyze
    pub path: PathBuf,

    /// Specific function to analyze
    pub function: Option<String>,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

### Output Types

```rust
/// Reference to an expression in source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpressionRef {
    pub text: String,
    pub line: u32,
    pub value_number: u32,
}

/// A group of expressions sharing the same value number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GVNEquivalence {
    pub value_number: u32,
    pub expressions: Vec<ExpressionRef>,
    #[serde(default)]
    pub reason: String,
}

/// A redundant expression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Redundancy {
    pub original: ExpressionRef,
    pub redundant: ExpressionRef,
    #[serde(default)]
    pub reason: String,
}

/// GVN summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GVNSummary {
    pub total_expressions: u32,
    pub unique_values: u32,
    pub compression_ratio: f64,
}

/// GVN report for a function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GVNReport {
    pub function: String,
    #[serde(default)]
    pub equivalences: Vec<GVNEquivalence>,
    #[serde(default)]
    pub redundancies: Vec<Redundancy>,
    pub summary: GVNSummary,
}
```

### Behavioral Contract

1. Hash-based value numbering for expressions
2. Handles commutative operators (`a+b` == `b+a`)
3. Tracks variable assignments to propagate values
4. Reports redundant expressions (same value, different location)

### Algorithm

```rust
// Pseudocode for GVN
fn compute_gvn(source: &str) -> Vec<GVNReport> {
    let ast = parse(source);
    let mut reports = vec![];
    
    for func in ast.functions() {
        let mut engine = GVNEngine::new();
        for expr in func.expressions() {
            let (hash, is_commutative) = engine.hash_expr(expr);
            let vn = engine.get_or_create_vn(hash);
            engine.record(expr, vn);
        }
        reports.push(engine.to_report());
    }
    reports
}
```

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error |

### Dependencies

- AST extractor
- Expression hasher (commutative-aware)
- Variable tracking

### Complexity Estimate: **HIGH**

GVN algorithm with commutative handling and variable tracking is non-trivial.

---

## 10. vuln

**Purpose:** Vulnerability detection via taint analysis.

**Python Source:** `vuln_cmd.py`

### CLI Interface

```
tldr vuln <path> [OPTIONS]

Arguments:
  <path>  File or directory to analyze (required)

Options:
  --severity <LEVEL>            Filter by minimum severity [values: critical, high, medium, low]
  --vuln-type <TYPE>            Filter by vulnerability type
  --include-informational       Include info-level findings
  --lang <LANG>                 Language hint [default: auto]
  -f, --format <FORMAT>         Output format [default: json] [values: json, text, sarif]
```

### Args Struct

```rust
#[derive(Debug, Args)]
pub struct VulnArgs {
    /// File or directory to analyze
    pub path: PathBuf,

    /// Filter by minimum severity
    #[arg(long)]
    pub severity: Option<Severity>,

    /// Filter by vulnerability type
    #[arg(long)]
    pub vuln_type: Option<VulnType>,

    /// Include info-level findings
    #[arg(long)]
    pub include_informational: bool,

    /// Language hint
    #[arg(long, default_value = "auto")]
    pub lang: String,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

### Output Types

```rust
/// Types of vulnerabilities detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum VulnType {
    SqlInjection,
    Xss,
    CommandInjection,
    Ssrf,
    PathTraversal,
    Deserialization,
    Xxe,
    OpenRedirect,
    LdapInjection,
    XpathInjection,
}

impl VulnType {
    pub fn cwe_id(&self) -> &'static str {
        match self {
            Self::SqlInjection => "CWE-89",
            Self::Xss => "CWE-79",
            Self::CommandInjection => "CWE-78",
            Self::Ssrf => "CWE-918",
            Self::PathTraversal => "CWE-22",
            Self::Deserialization => "CWE-502",
            Self::Xxe => "CWE-611",
            Self::OpenRedirect => "CWE-601",
            Self::LdapInjection => "CWE-90",
            Self::XpathInjection => "CWE-643",
        }
    }
    
    pub fn default_severity(&self) -> Severity {
        match self {
            Self::SqlInjection | Self::CommandInjection | Self::Deserialization => Severity::Critical,
            Self::Xxe | Self::Xss | Self::Ssrf | Self::PathTraversal 
                | Self::LdapInjection | Self::XpathInjection => Severity::High,
            Self::OpenRedirect => Severity::Medium,
        }
    }
}

/// A step in the taint flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintFlow {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub code_snippet: String,
    pub description: String,
}

/// A vulnerability finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnFinding {
    pub vuln_type: VulnType,
    pub severity: Severity,
    pub cwe_id: String,
    pub title: String,
    pub description: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub taint_flow: Vec<TaintFlow>,
    pub remediation: String,
    pub confidence: f64,
}

/// Vulnerability summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VulnSummary {
    pub total_findings: u32,
    pub by_severity: HashMap<String, u32>,
    pub by_type: HashMap<String, u32>,
    pub files_with_vulns: u32,
}

/// Vulnerability report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnReport {
    pub findings: Vec<VulnFinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<VulnSummary>,
    pub scan_duration_ms: u64,
    pub files_scanned: u32,
}
```

### Behavioral Contract

1. Taint sources: User input (request.args, request.form, etc.)
2. Taint sinks: Dangerous functions (cursor.execute, os.system, etc.)
3. Track data flow from sources to sinks
4. Check for sanitization along the way
5. Report vulnerability with full taint flow

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success, no vulnerabilities |
| 1 | Error |
| 2 | Vulnerabilities found |

### Dependencies

- AST extractor
- Taint tracker
- Source/sink patterns (per framework)
- Sanitizer detection

### Complexity Estimate: **HIGH**

Full taint analysis with source-sink tracking across variables.

---

## Security Mitigations

### TIGER (Critical)

| ID | Risk | Mitigation |
|----|------|------------|
| T2 | Path traversal | `validate_file_path`, canonicalize |
| T3 | Unbounded loops | Max file count, max depth |
| T4 | Memory exhaustion | Max file size |
| T8 | Stack overflow | Max AST depth |

### Resource Limits

```rust
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;  // 10 MB
pub const MAX_DIRECTORY_FILES: u32 = 1000;
pub const MAX_ANALYSIS_DEPTH: usize = 500;
pub const MAX_AST_DEPTH: usize = 100;
```

---

## Implementation Phases

### Phase 1: Foundation + LOW Commands
**Files:** `mod.rs`, `types.rs`, `error.rs`, `validation.rs`, `todo.rs`, `explain.rs`, `secure.rs`

**Estimated effort:** 1-2 days

### Phase 2: MEDIUM Commands
**Files:** `definition.rs`, `references.rs`, `diff.rs`, `diff_impact.rs`

**Estimated effort:** 3-4 days

### Phase 3: HIGH Commands
**Files:** `api_check.rs`, `equivalence.rs`, `vuln.rs`

**Estimated effort:** 5-7 days

---

## Success Criteria

1. All 9 commands pass `--help` and produce valid JSON
2. `tldr todo` aggregates and sorts improvement items
3. `tldr explain` produces complete function analysis
4. `tldr secure` runs all security sub-analyses
5. `tldr definition` finds cross-file definitions
6. `tldr diff` detects moves and renames
7. `tldr api-check` detects all Python rules
8. `tldr equivalence` finds redundant expressions
9. `tldr vuln` detects injection vulnerabilities with taint flow
10. All TIGER mitigations pass security review
