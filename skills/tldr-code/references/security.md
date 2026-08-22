# Security Commands

Security commands detect vulnerabilities, taint flows, and API misuse.

## taint

**Alias:** `ta`

**Purpose:** Analyze taint flows to detect security vulnerabilities.

**Implementation:** `crates/tldr-cli/src/commands/taint.rs`

```rust
pub struct TaintArgs {
    pub file: PathBuf,
    pub function: String,
    pub lang: Option<Language>,
    pub verbose: bool,
}
```

**How it works:**
1. Builds CFG and DFG for function
2. Marks sources as tainted (user input, files, network)
3. Propagates taint through operations
4. Checks for sanitizers along paths
5. Reports taint flows to sensitive sinks

**Taint sources:**
- Function parameters
- File reads
- Network input
- Environment variables

**Taint sinks:**
- SQL queries (`execute`, `query`)
- Command execution (`exec`, `system`)
- File operations (`open`, `write`)
- HTML/JS output (`innerHTML`, `document.write`)

**Example:**
```bash
tldr taint src/process.py handle_request

# Verbose output
tldr taint src/process.py handle_request -v
```

---

## vuln

**Purpose:** Vulnerability scanning via taint analysis.

**Implementation:** `crates/tldr-cli/src/commands/remaining/vuln.rs`

**How it works:**
1. Scans all functions in scope
2. Runs taint analysis per function
3. Categorizes by vulnerability type
4. Filters by severity

**Vulnerability types:**
- `sql_injection` — Unescaped SQL
- `xss` — Unescaped HTML/JS output
- `command_injection` — Unsanitized command execution
- `ssrf` — Server-side request forgery
- `path_traversal` — Unsanitized file paths
- `deserialization` — Unsafe deserialization
- `unsafe_code` — Memory unsafe operations
- `memory_safety` — Buffer overflow, use-after-free
- And more...

**Example:**
```bash
tldr vuln src/

# High severity only
tldr vuln src/ --severity high

# Specific type
tldr vuln src/ --vuln-type sql_injection
```

---

## secure

**Alias:** `sec`

**Purpose:** Security analysis dashboard (aggregate of multiple analyses).

**Implementation:** `crates/tldr-cli/src/commands/remaining/secure.rs`

**How it works:**
1. Runs multiple security analyses:
   - `taint` — Taint flow analysis
   - `resources` — Resource lifecycle
   - `bounds` — Buffer bounds
   - `contracts` — Pre/postcondition violations
   - `behavioral` — Behavioral patterns
   - `mutability` — Mutable state issues
2. Aggregates into security score

**Example:**
```bash
tldr secure src/

# Quick mode
tldr secure src/ --quick

# Detail specific sub-analysis
tldr secure src/ --detail taint
```

---

## api-check

**Alias:** `ac`

**Purpose:** Detect API misuse patterns.

**Implementation:** `crates/tldr-cli/src/commands/remaining/api_check.rs`

**Categories:**
- `call-order` — Wrong sequence (e.g., use before init)
- `error-handling` — Missing try/catch, bare except
- `parameters` — Wrong types, missing required
- `resources` — Unclosed files, unclosed connections
- `crypto` — Weak crypto, missing IVs
- `concurrency` — Race conditions, deadlocks
- `security` — Auth bypasses, etc.

**Example:**
```bash
tldr api-check src/

# Specific category
tldr api-check src/ --category error-handling

# Severity filter
tldr api-check src/ --severity high
```

---

## resources

**Alias:** `res`

**Purpose:** Analyze resource lifecycle (leaks, double-close, use-after-close).

**Implementation:** `crates/tldr-cli/src/commands/patterns/resources.rs`

**Checks:**
- **R2**: Memory/file descriptor leaks
- **R3**: Double-close detection
- **R4**: Use-after-close
- **R6**: Suggest context manager usage
- **R7**: Detailed leak paths

**Example:**
```bash
tldr resources src/database.py

# All checks
tldr resources src/database.py --check-all

# Leak paths
tldr resources src/database.py --show-paths

# With suggestions
tldr resources src/database.py --suggest-context
```
