# Contracts & Flow Commands - Rust Port Specification

**Created:** 2026-02-03
**Author:** architect-agent
**Source:** Python v1 at `/Users/cosimo/.opc-dev/opc/packages/tldr-code/tldr/`
**Target:** `/Users/cosimo/.opc-dev/opc/packages/tldr-code/tldr-rs/crates/tldr-cli/src/commands/contracts/`

## Overview

This specification defines the Rust port of 7 TLDR commands for contract inference, behavioral specification extraction, and program flow analysis. These commands help users understand function contracts, invariants, and data flow paths through their code.

The port follows established patterns from the existing Rust CLI (e.g., `slice.rs`, `daemon/` module organization).

## Module Architecture

```
contracts/
├── mod.rs              # Module exports and re-exports
├── error.rs            # ContractsError enum and Result type
├── types.rs            # Shared data types across all commands
├── contracts.rs        # contracts command - pre/postcondition inference
├── invariants.rs       # invariants command - Daikon-lite inference
├── specs.rs            # specs command - test-derived specifications
├── verify.rs           # verify command - aggregated verification dashboard
├── dead_stores.rs      # dead-stores command - SSA-based dead store detection
├── bounds.rs           # bounds command - interval analysis
└── chop.rs             # chop command - program slice intersection
```

## Shared Types (`types.rs`)

### Confidence Level

```rust
use serde::{Deserialize, Serialize};

/// Confidence level for inferred contracts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Direct code evidence (guard clause, assertion)
    High,
    /// Inferred from patterns or types
    Medium,
    /// Derived from type hints only
    Low,
}

impl Default for Confidence {
    fn default() -> Self {
        Self::Medium
    }
}
```

### Condition (Contract Element)

```rust
/// A single contract condition (precondition, postcondition, or invariant)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    /// Variable name this condition applies to
    pub variable: String,
    
    /// Human-readable constraint expression (e.g., "x > 0", "isinstance(x, str)")
    pub constraint: String,
    
    /// Source line where condition was detected
    pub source_line: u32,
    
    /// Confidence level
    pub confidence: Confidence,
}
```

### Invariant Kind

```rust
/// Types of invariants that can be inferred
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantKind {
    /// Type invariant (e.g., "x: int")
    Type,
    /// Non-null invariant
    NonNull,
    /// Non-negative numeric
    NonNegative,
    /// Positive numeric
    Positive,
    /// Range constraint
    Range,
    /// Ordering relation between parameters
    Relation,
    /// Length constraint
    Length,
}
```

### Spec Types

```rust
/// Input/Output specification from test assertion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputOutputSpec {
    pub function: String,
    pub inputs: Vec<serde_json::Value>,
    pub output: serde_json::Value,
    pub test_function: String,
    pub line: u32,
    pub confidence: Confidence,
}

/// Exception specification from pytest.raises
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExceptionSpec {
    pub function: String,
    pub inputs: Vec<serde_json::Value>,
    pub exception_type: String,
    pub match_pattern: Option<String>,
    pub test_function: String,
    pub line: u32,
    pub confidence: Confidence,
}

/// Property specification (type, length, bounds)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertySpec {
    pub function: String,
    /// "type", "length", "bounds", "boolean", "membership", "not_none"
    pub property_type: String,
    pub constraint: String,
    pub test_function: String,
    pub line: u32,
    pub confidence: Confidence,
}
```

### Interval

```rust
/// Numeric interval [lo, hi] for bounds analysis
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Interval {
    /// Lower bound (f64::NEG_INFINITY for unbounded)
    pub lo: f64,
    /// Upper bound (f64::INFINITY for unbounded)
    pub hi: f64,
}

impl Interval {
    pub fn const_val(n: f64) -> Self {
        Self { lo: n, hi: n }
    }
    
    pub fn top() -> Self {
        Self { lo: f64::NEG_INFINITY, hi: f64::INFINITY }
    }
    
    pub fn bottom() -> Self {
        Self { lo: f64::INFINITY, hi: f64::NEG_INFINITY }
    }
    
    pub fn is_bottom(&self) -> bool {
        self.lo > self.hi
    }
    
    pub fn contains(&self, n: f64) -> bool {
        !self.is_bottom() && self.lo <= n && n <= self.hi
    }
}
```

## Error Types (`error.rs`)

```rust
use std::path::PathBuf;
use thiserror::Error;

/// Errors specific to contracts and flow analysis commands
#[derive(Debug, Error)]
pub enum ContractsError {
    /// Source file not found
    #[error("file not found: {}", path.display())]
    FileNotFound { path: PathBuf },
    
    /// Function not found in source
    #[error("function '{function}' not found in {}", file.display())]
    FunctionNotFound { function: String, file: PathBuf },
    
    /// Test path not found
    #[error("test path not found: {}", path.display())]
    TestPathNotFound { path: PathBuf },
    
    /// Line outside function range
    #[error("line {line} is outside function '{function}' (lines {start}-{end})")]
    LineOutsideFunction {
        line: u32,
        function: String,
        start: u32,
        end: u32,
    },
    
    /// Parse error in source file
    #[error("parse error in {}: {message}", file.display())]
    ParseError { file: PathBuf, message: String },
    
    /// SSA construction failed
    #[error("SSA construction failed: {0}")]
    SsaError(String),
    
    /// Analysis did not converge
    #[error("analysis did not converge after {iterations} iterations")]
    DidNotConverge { iterations: u32 },
    
    /// Sub-analysis failed in verify command
    #[error("sub-analysis '{name}' failed: {message}")]
    SubAnalysisFailed { name: String, message: String },
    
    /// No test directory found
    #[error("no test directory found in {}", project.display())]
    NoTestDirectory { project: PathBuf },
    
    /// Generic IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    /// JSON serialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Result type for contracts commands
pub type ContractsResult<T> = Result<T, ContractsError>;
```

---

## Command Specifications

### 1. contracts

**Purpose:** Infer pre/postconditions from guard clauses, assertions, and isinstance checks.

#### CLI Interface

```
tldr contracts <file> <function> [OPTIONS]

Arguments:
  <file>      Source file to analyze (required)
  <function>  Function name to analyze (required)

Options:
  -f, --format <FORMAT>  Output format [default: json] [possible values: json, text]
  -l, --lang <LANG>      Language override (auto-detected if not specified)
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct ContractsArgs {
    /// Source file to analyze
    pub file: PathBuf,
    
    /// Function name to analyze
    pub function: String,
    
    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
    
    /// Language override
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}
```

#### Output Type

```rust
#[derive(Debug, Serialize)]
pub struct ContractsReport {
    pub function: String,
    pub file: PathBuf,
    pub preconditions: Vec<Condition>,
    pub postconditions: Vec<Condition>,
    pub invariants: Vec<Condition>,
}
```

#### Behavioral Contracts

**Preconditions (requires):**
- `file` must exist and be readable
- `function` must be a valid identifier

**Postconditions (ensures):**
- Returns `ContractsReport` with detected conditions
- Each condition has a valid `source_line` within the function
- Empty vectors are valid (no contracts detected)

**Error Conditions:**
| Condition | Error |
|-----------|-------|
| File does not exist | `ContractsError::FileNotFound` |
| Function not in file | `ContractsError::FunctionNotFound` |
| Parse failure | `ContractsError::ParseError` |

#### Detection Patterns

| Pattern | Confidence | Example |
|---------|------------|---------|
| `if <cond>: raise` | High | `if x < 0: raise ValueError` -> precond: `x >= 0` |
| `assert <cond>` | High | `assert isinstance(x, str)` -> precond: `isinstance(x, str)` |
| `if not isinstance(...)`: raise | High | Direct type precondition |
| Assert after `result =` | High | Postcondition on result |
| Type annotations | Low | `def f(x: int)` -> precond: `isinstance(x, int)` |

#### Text Output Format

```
Function: process_data
  Preconditions:
    - x >= 0 (x, line 10, high)
    - isinstance(data, list) (data, line 11, high)
  Postconditions:
    - result is not None (result, line 25, high)
  Invariants:
    (none detected)
```

---

### 2. invariants

**Purpose:** Daikon-lite inference of likely invariants from test execution traces.

#### CLI Interface

```
tldr invariants <file> --from-tests <test_path> [OPTIONS]

Arguments:
  <file>  Source file containing functions to analyze (required)

Options:
  -t, --from-tests <PATH>    Test file or directory (required)
  -f, --format <FORMAT>      Output format [default: json]
  --function <NAME>          Filter to specific function
  --min-obs <N>              Minimum observations to report [default: 1]
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct InvariantsArgs {
    /// Source file to analyze
    pub file: PathBuf,
    
    /// Test file or directory for tracing
    #[arg(long = "from-tests", short = 't')]
    pub from_tests: PathBuf,
    
    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
    
    /// Filter to specific function
    #[arg(long)]
    pub function: Option<String>,
    
    /// Minimum observations required
    #[arg(long, default_value = "1")]
    pub min_obs: u32,
}
```

#### Output Type

```rust
#[derive(Debug, Serialize)]
pub struct Invariant {
    pub variable: String,
    pub kind: InvariantKind,
    pub expression: String,
    pub confidence: Confidence,
    pub observations: u32,
    pub counterexample_count: u32,
}

#[derive(Debug, Serialize)]
pub struct FunctionInvariants {
    pub function_name: String,
    pub preconditions: Vec<Invariant>,
    pub postconditions: Vec<Invariant>,
    pub observation_count: u32,
}

#[derive(Debug, Serialize)]
pub struct InvariantsReport {
    pub functions: Vec<FunctionInvariants>,
    pub summary: InvariantsSummary,
}

#[derive(Debug, Serialize)]
pub struct InvariantsSummary {
    pub total_observations: u32,
    pub total_invariants: u32,
    pub by_kind: HashMap<String, u32>,
}
```

#### Behavioral Contracts

**Preconditions:**
- `file` must exist and be valid source
- `from_tests` must exist (file or directory)

**Postconditions:**
- Returns invariants for all functions with observations >= `min_obs`
- Confidence is calculated: low (<5 obs), medium (5-9), high (10+)

**Error Conditions:**
| Condition | Error |
|-----------|-------|
| Source file not found | `ContractsError::FileNotFound` |
| Test path not found | `ContractsError::TestPathNotFound` |

#### Invariant Detection Rules

| Kind | Detection Rule | Example |
|------|---------------|---------|
| Type | All values same type | `x: int` |
| NonNull | No None values observed | `x is not None` |
| NonNegative | All numeric values >= 0 | `x >= 0` |
| Positive | All numeric values > 0 | `x > 0` |
| Range | Track min/max observed | `0 <= x <= 100` |
| Relation | p1 < p2 for all observations | `start < end` |

#### Text Output Format

```
Function: calculate (15 observations)
  Requires: x: int [high]
  Requires: x >= 0 [high]
  Ensures: result: float [high]
  Ensures: result >= 0 [medium]
```

---

### 3. specs

**Purpose:** Extract behavioral specifications from pytest test files.

#### CLI Interface

```
tldr specs --from-tests <test_path> [OPTIONS]

Options:
  -t, --from-tests <PATH>    Test file or directory (required)
  -f, --format <FORMAT>      Output format [default: json]
  --function <NAME>          Filter to specific function under test
  --source <PATH>            Source directory for cross-referencing
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct SpecsArgs {
    /// Test file or directory
    #[arg(long = "from-tests", short = 't')]
    pub from_tests: PathBuf,
    
    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
    
    /// Filter to specific function
    #[arg(long)]
    pub function: Option<String>,
    
    /// Source directory for cross-referencing
    #[arg(long)]
    pub source: Option<PathBuf>,
}
```

#### Output Type

```rust
#[derive(Debug, Serialize)]
pub struct FunctionSpecs {
    pub function_name: String,
    pub summary: String,  // e.g., "3 input/output, 1 raises"
    pub test_count: u32,
    pub input_output_specs: Vec<InputOutputSpec>,
    pub exception_specs: Vec<ExceptionSpec>,
    pub property_specs: Vec<PropertySpec>,
}

#[derive(Debug, Serialize)]
pub struct SpecsReport {
    pub functions: Vec<FunctionSpecs>,
    pub summary: SpecsSummary,
}

#[derive(Debug, Serialize)]
pub struct SpecsSummary {
    pub total_specs: u32,
    pub by_type: SpecsByType,
    pub test_functions_scanned: u32,
    pub test_files_scanned: u32,
    pub functions_found: u32,
}

#[derive(Debug, Serialize)]
pub struct SpecsByType {
    pub input_output: u32,
    pub exception: u32,
    pub property: u32,
}
```

#### Behavioral Contracts

**Preconditions:**
- `from_tests` path must exist

**Postconditions:**
- Scans all `test_*.py` files recursively
- Extracts specs from `assert func(args) == expected`
- Extracts specs from `with pytest.raises(Exception)`
- Extracts property specs from `isinstance`, `len()` assertions

**Error Conditions:**
| Condition | Error |
|-----------|-------|
| Path not found | `ContractsError::TestPathNotFound` |

#### Extraction Patterns

| Pattern | Spec Type | Example |
|---------|-----------|---------|
| `assert f(x) == y` | InputOutput | `add(2, 3) == 5` |
| `with pytest.raises(E)` | Exception | `raises(ValueError)` |
| `assert isinstance(f(x), T)` | Property (type) | `isinstance(result, list)` |
| `assert len(f(x)) == n` | Property (length) | `len(result) == 3` |
| `assert f(x) > n` | Property (bounds) | `result > 0` |
| `assert "key" in f(x)` | Property (membership) | `"id" in result` |

#### Text Output Format

```
Function: add
  IO: add(2, 3) == 5
  IO: add(-1, 1) == 0
  Raises: ValueError

Function: parse
  Property (type): isinstance(result, dict)
  Property (length): len(result) == 2

Total specs: 5
```

---

### 4. verify

**Purpose:** Aggregated verification dashboard combining multiple analyses.

#### CLI Interface

```
tldr verify [path] [OPTIONS]

Arguments:
  [path]  Directory to analyze [default: .]

Options:
  -f, --format <FORMAT>    Output format [default: json]
  -l, --lang <LANG>        Language override
  --detail <ANALYSIS>      Show specific sub-analysis result
  --quick                  Quick mode (skip invariants and patterns)
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// Directory to analyze
    #[arg(default_value = ".")]
    pub path: PathBuf,
    
    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
    
    /// Language override
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
    
    /// Show specific sub-analysis
    #[arg(long)]
    pub detail: Option<String>,
    
    /// Quick mode (skip slow analyses)
    #[arg(long)]
    pub quick: bool,
}
```

#### Output Type

```rust
#[derive(Debug, Serialize)]
pub struct SubAnalysisResult {
    pub name: String,
    pub success: bool,
    pub error: Option<String>,
    pub elapsed_ms: u64,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct VerifyReport {
    pub path: PathBuf,
    pub sub_results: HashMap<String, SubAnalysisResult>,
    pub summary: VerifySummary,
    pub total_elapsed_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct VerifySummary {
    pub spec_count: u32,
    pub invariant_count: u32,
    pub contract_count: u32,
    pub annotated_count: u32,
    pub behavioral_count: u32,
    pub pattern_count: u32,
    pub pattern_high_confidence: u32,
    pub coverage: CoverageInfo,
}

#[derive(Debug, Serialize)]
pub struct CoverageInfo {
    pub constrained_functions: u32,
    pub total_functions: u32,
    pub coverage_pct: f64,
}
```

#### Behavioral Contracts

**Sub-analyses run:**
1. `contracts` - Contract inference on all files
2. `behavioral` - Behavioral model extraction
3. `annotated` - Annotated[T] constraint extraction
4. `specs` - Test spec extraction (if test dir found)
5. `invariants` - Daikon-lite (skipped in quick mode)
6. `patterns` - Pattern mining (skipped in quick mode)

**Postconditions:**
- Each sub-analysis reports success/failure independently
- Coverage percentage calculated from constrained vs total functions
- Maximum 500 files analyzed

**Error Conditions:**
- Individual sub-analysis failures are captured, not propagated

#### Text Output Format

```
Verification: ./src
==================================================
Test Specs:    42 behavioral specs extracted
Invariants:    15 inferred invariants
Contracts:     28 pre/postconditions inferred
Annotations:   5 Annotated[T] constraints found
Behaviors:     18 functions with behavioral models
Patterns:      3 project patterns (2 high-confidence)

Constraint Coverage:
  Functions with any constraint: 45/60 (75.0%)

Elapsed: 1234ms
```

---

### 5. dead-stores

**Purpose:** SSA-based dead store detection - find assignments that are never read.

#### CLI Interface

```
tldr dead-stores <file> <function> [OPTIONS]

Arguments:
  <file>      Source file to analyze (required)
  <function>  Function name to analyze (required)

Options:
  -l, --lang <LANG>     Language override
  --compare             Compare with live-vars based detection
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct DeadStoresArgs {
    /// Source file to analyze
    pub file: PathBuf,
    
    /// Function name to analyze
    pub function: String,
    
    /// Language override
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
    
    /// Compare SSA vs liveness-based detection
    #[arg(long)]
    pub compare: bool,
}
```

#### Output Type

```rust
#[derive(Debug, Serialize)]
pub struct DeadStore {
    /// Variable name that was assigned
    pub variable: String,
    /// SSA version name (e.g., "x_2")
    pub ssa_name: String,
    /// Line number of the dead assignment
    pub line: u32,
    /// Block ID where assignment occurs
    pub block_id: u32,
    /// Whether this is a phi function definition
    pub is_phi: bool,
}

impl DeadStore {
    pub fn to_dict(&self) -> serde_json::Value {
        serde_json::json!({
            "variable": self.variable,
            "ssa_name": self.ssa_name,
            "line": self.line,
            "block_id": self.block_id,
            "is_phi": self.is_phi,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct DeadStoresReport {
    pub function: String,
    pub file: PathBuf,
    pub dead_stores_ssa: Vec<DeadStore>,
    pub count: u32,
    /// Only present if --compare flag used
    pub dead_stores_live_vars: Option<Vec<serde_json::Value>>,
    pub live_vars_count: Option<u32>,
}
```

#### Core Algorithm: `find_dead_stores_ssa`

**THIS FUNCTION MUST BE IMPLEMENTED** (missing in Python v1)

```rust
/// Find dead stores in SSA form.
/// 
/// A definition is dead if it has no uses (empty use_sites).
/// Phi function definitions without uses may be normal in some cases.
/// 
/// # Arguments
/// * `ssa_info` - SSA analysis results with def_sites and use_sites
/// 
/// # Returns
/// List of DeadStore instances for each dead definition
pub fn find_dead_stores_ssa(ssa_info: &SsaInfo) -> Vec<DeadStore> {
    let mut dead_stores = Vec::new();
    
    for (ssa_name, def_site) in &ssa_info.def_sites {
        // Get use sites for this definition
        let uses = ssa_info.use_sites.get(ssa_name);
        
        // If no uses, it's a dead store
        if uses.map_or(true, |u| u.is_empty()) {
            // Extract original variable name (strip version suffix)
            let original_var = ssa_name
                .rsplit('_')
                .skip(1)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("_");
            
            let is_phi = def_site.get("is_phi")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            
            // Skip phi functions without uses (may be normal at loop exits)
            // User can decide whether to filter these
            
            dead_stores.push(DeadStore {
                variable: if original_var.is_empty() { 
                    ssa_name.clone() 
                } else { 
                    original_var 
                },
                ssa_name: ssa_name.clone(),
                line: def_site.get("line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                block_id: def_site.get("block")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                is_phi,
            });
        }
    }
    
    dead_stores
}
```

#### Behavioral Contracts

**Preconditions:**
- File must exist and be parseable
- Function must exist in file
- SSA construction must succeed

**Postconditions:**
- Returns all definitions with empty use_sites
- Phi functions are flagged separately

**Error Conditions:**
| Condition | Error |
|-----------|-------|
| File not found | `ContractsError::FileNotFound` |
| Function not found | `ContractsError::FunctionNotFound` |
| SSA construction failed | `ContractsError::SsaError` |

---

### 6. bounds

**Purpose:** Interval analysis tracking numeric value ranges through code.

#### CLI Interface

```
tldr bounds <file> [function] [OPTIONS]

Arguments:
  <file>      Source file to analyze (required)
  [function]  Function name (analyzes all if not specified)

Options:
  -f, --format <FORMAT>    Output format [default: json]
  --max-iter <N>           Maximum fixpoint iterations [default: 50]
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct BoundsArgs {
    /// Source file to analyze
    pub file: PathBuf,
    
    /// Function name (optional)
    pub function: Option<String>,
    
    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
    
    /// Maximum iterations for fixpoint
    #[arg(long, default_value = "50")]
    pub max_iter: u32,
}
```

#### Output Type

```rust
#[derive(Debug, Serialize)]
pub struct IntervalWarning {
    pub line: u32,
    pub kind: String,  // "division_by_zero", "out_of_bounds"
    pub variable: String,
    pub bounds: Interval,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct BoundsResult {
    pub function: String,
    pub bounds: HashMap<u32, HashMap<String, Interval>>,  // line -> var -> interval
    pub warnings: Vec<IntervalWarning>,
    pub converged: bool,
    pub iterations: u32,
}
```

#### Behavioral Contracts

**Preconditions:**
- File must exist

**Postconditions:**
- Returns interval bounds for each variable at each line
- Detects potential division by zero (divisor interval contains 0)
- Converges within max_iter or sets `converged = false`

**Interval Lattice Operations:**
- Join: `[a,b] | [c,d] = [min(a,c), max(b,d)]`
- Meet: `[a,b] & [c,d] = [max(a,c), min(b,d)]` or bottom
- Widen: `[a,b] W [c,d] = [c<a ? -inf : a, d>b ? +inf : b]`

**Error Conditions:**
| Condition | Error |
|-----------|-------|
| File not found | `ContractsError::FileNotFound` |
| Did not converge | `ContractsError::DidNotConverge` (warning, still returns result) |

#### Text Output Format

```
Function: calculate
  Converged: true (12 iterations)
  Line 10: x in [0, 100]
  Line 11: y in [1, 50]
  Line 15: result in [0, inf]
  WARNING [division_by_zero] line 20: Divisor may be zero: x / y in [0, 50]
```

---

### 7. chop

**Purpose:** Compute the intersection of forward and backward slices.

#### CLI Interface

```
tldr chop <file> <function> <source_line> <target_line> [OPTIONS]

Arguments:
  <file>         Source file to analyze (required)
  <function>     Function containing both lines (required)
  <source_line>  Line to trace FROM (required)
  <target_line>  Line to trace TO (required)

Options:
  -l, --lang <LANG>  Language override
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct ChopArgs {
    /// Source file to analyze
    pub file: PathBuf,
    
    /// Function name
    pub function: String,
    
    /// Line to trace from
    pub source_line: u32,
    
    /// Line to trace to
    pub target_line: u32,
    
    /// Language override
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}
```

#### Output Type

```rust
#[derive(Debug, Serialize)]
pub struct ChopResult {
    /// Lines on the dependency path (sorted)
    pub lines: Vec<u32>,
    pub count: u32,
    pub source_line: u32,
    pub target_line: u32,
    /// True if source_line is in backward_slice(target_line)
    pub path_exists: bool,
    pub function: String,
    /// Human-readable explanation
    pub explanation: Option<String>,
}
```

#### Core Algorithm

```rust
/// Compute chop slice: forward_slice(source) ∩ backward_slice(target)
/// 
/// # Algorithm
/// 1. Compute forward_slice(source_line) - statements source can affect
/// 2. Compute backward_slice(target_line) - statements that can affect target
/// 3. path_exists = source_line ∈ backward_slice(target_line)
/// 4. chop = forward ∩ backward
pub fn chop(
    source_or_path: &str,
    function_name: &str,
    source_line: u32,
    target_line: u32,
    language: Language,
) -> ContractsResult<ChopResult> {
    // Same line case
    if source_line == target_line {
        return Ok(ChopResult {
            lines: vec![source_line],
            count: 1,
            source_line,
            target_line,
            path_exists: true,
            function: function_name.to_string(),
            explanation: Some(format!(
                "Source and target are the same line ({}).", 
                source_line
            )),
        });
    }
    
    // Extract PDG
    let pdg = extract_pdg(source_or_path, function_name, language)?;
    
    // Validate lines are in function range
    let (func_start, func_end) = pdg.line_range();
    if source_line < func_start || source_line > func_end {
        return Err(ContractsError::LineOutsideFunction {
            line: source_line,
            function: function_name.to_string(),
            start: func_start,
            end: func_end,
        });
    }
    if target_line < func_start || target_line > func_end {
        return Err(ContractsError::LineOutsideFunction {
            line: target_line,
            function: function_name.to_string(),
            start: func_start,
            end: func_end,
        });
    }
    
    // Compute slices
    let forward = pdg.forward_slice(source_line);
    let backward = pdg.backward_slice(target_line);
    
    // Check path existence
    let path_exists = backward.contains(&source_line);
    
    if !path_exists {
        return Ok(ChopResult {
            lines: vec![],
            count: 0,
            source_line,
            target_line,
            path_exists: false,
            function: function_name.to_string(),
            explanation: Some(format!(
                "No dependency path from line {} to line {}. \
                 The source line does not affect the target line.",
                source_line, target_line
            )),
        });
    }
    
    // Compute intersection
    let chop_lines: HashSet<u32> = forward.intersection(&backward).copied().collect();
    let mut lines: Vec<u32> = chop_lines.into_iter().collect();
    lines.sort();
    
    Ok(ChopResult {
        count: lines.len() as u32,
        lines,
        source_line,
        target_line,
        path_exists: true,
        function: function_name.to_string(),
        explanation: Some(format!(
            "Found {} lines on the dependency path from line {} to line {}.",
            lines.len(), source_line, target_line
        )),
    })
}
```

#### Behavioral Contracts

**Preconditions:**
- File must exist
- Function must exist in file
- Both lines must be within function range

**Postconditions:**
- If `path_exists` is false, `lines` is empty
- If `path_exists` is true, both `source_line` and `target_line` are in `lines`
- `lines` is always sorted

**Invariants:**
- `lines` is subset of `forward_slice(source_line)`
- `lines` is subset of `backward_slice(target_line)`

**Error Conditions:**
| Condition | Error |
|-----------|-------|
| File not found | `ContractsError::FileNotFound` |
| Function not found | `ContractsError::FunctionNotFound` |
| Line outside function | `ContractsError::LineOutsideFunction` |

---

## Integration Points

### Dependencies on Other Crates

```toml
# tldr-cli/Cargo.toml additions
[dependencies]
tldr-core = { path = "../tldr-core" }
```

### Required from tldr-core

These functions/types must be available or ported:

```rust
// From tldr-core
pub use tldr_core::{
    // PDG extraction
    extract_pdg, PdgInfo,
    // CFG extraction  
    extract_cfg, CfgInfo,
    // SSA construction
    compute_ssa, SsaInfo, PhiFunction,
    // Slice operations
    forward_slice, backward_slice,
    // Language detection
    Language,
};
```

### Module Registration

Add to `commands/mod.rs`:

```rust
// Contracts & Flow commands (Session 18)
pub mod contracts;

// Re-export Args types
pub use contracts::{
    ContractsArgs, InvariantsArgs, SpecsArgs, VerifyArgs,
    DeadStoresArgs, BoundsArgs, ChopArgs,
};
```

---

## Implementation Phases

### Phase 1: Foundation (types.rs, error.rs, mod.rs)
- Define all shared types
- Define error enum
- Set up module structure

**Acceptance Criteria:**
- [ ] Types compile without errors
- [ ] Error enum covers all failure modes
- [ ] Module re-exports work

### Phase 2: Core Commands (contracts.rs, specs.rs, chop.rs)
- Implement contracts command
- Implement specs command  
- Implement chop command

**Acceptance Criteria:**
- [ ] `tldr contracts` works on Python files
- [ ] `tldr specs` extracts from pytest files
- [ ] `tldr chop` computes slice intersection

### Phase 3: Analysis Commands (bounds.rs, dead_stores.rs)
- Implement bounds (interval analysis)
- Implement dead-stores with `find_dead_stores_ssa`

**Acceptance Criteria:**
- [ ] `tldr bounds` tracks intervals through assignments
- [ ] `tldr dead-stores` finds unused assignments
- [ ] `find_dead_stores_ssa` correctly identifies dead defs

### Phase 4: Integration (verify.rs, invariants.rs)
- Implement verify dashboard
- Implement invariants (requires test execution integration)

**Acceptance Criteria:**
- [ ] `tldr verify` aggregates all sub-analyses
- [ ] Coverage percentage calculated correctly
- [ ] `tldr invariants` works with test tracing

### Phase 5: Testing & Documentation
- Unit tests for each command
- Integration tests with sample files
- Documentation updates

**Coverage Target:** 80%

---

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| `find_dead_stores_ssa` not implemented in Python | High | Implement from scratch using algorithm in spec |
| Test tracing for invariants complex | Medium | Start with static analysis, add tracing later |
| Interval analysis may not converge | Medium | Use widening after N iterations |
| Large files slow verify command | Medium | Maintain 500-file limit, add progress |

---

## Open Questions

- [ ] Should invariants command support languages other than Python?
- [ ] Should chop support thin slicing mode?
- [ ] What level of Annotated[T] support for verify?

---

## Success Criteria

1. All 7 commands pass `--help` and produce valid JSON output
2. `tldr contracts` detects guard clause patterns
3. `tldr specs` extracts specs from pytest assertions
4. `tldr chop` correctly identifies dependency paths
5. `tldr dead-stores` finds assignments with no uses
6. `tldr bounds` warns on potential division by zero
7. `tldr verify` produces coverage percentage
8. Error messages are actionable and specific
