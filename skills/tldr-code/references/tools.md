# Tools Commands

Miscellaneous tools for development workflow integration.

## doctor

**Alias:** `doc`

**Purpose:** Check and install diagnostic tools for each language.

**Implementation:** `crates/tldr-cli/src/commands/doctor.rs`

**How it works:**
1. Detects installed tools per language
2. Reports missing tools
3. Optionally installs via `--install`

**Example:**
```bash
tldr doctor

# Install tools
tldr doctor --install python
tldr doctor --install rust
tldr doctor --install go
```

**Supported languages and tools:**
- Python: pyright, ruff, mypy
- TypeScript: typescript-language-server, tsc
- Go: gopls, golangci-lint
- Rust: rustc, cargo
- Java: checkstyle, spotbugs

---

## diagnostics

**Alias:** `diag`

**Purpose:** Run type checking and linting using external tools.

**Implementation:** `crates/tldr-cli/src/commands/diagnostics.rs`

**Example:**
```bash
tldr diagnostics src/

# Specific tools
tldr diagnostics src/ --tools pyright,ruff

# Skip type checking (linters only)
tldr diagnostics src/ --no-typecheck

# Output for GitHub Actions
tldr diagnostics src/ --output github-actions
```

---

## fix

**Alias:** `fx`

**Purpose:** Diagnose and auto-fix errors from compiler/runtime output.

**Subcommands:**

### fix diagnose

Parse error output and produce structured diagnosis.

```bash
tldr fix diagnose "error: ..."
tldr fix diagnose < build.log
```

### fix apply

Apply fix edits to source code.

```bash
tldr fix apply < fix.json
```

### fix check

Run test command, diagnose failures, apply fixes, re-run in a loop.

```bash
tldr fix check -- cargo test
tldr fix check -- pytest
```

---

## bugbot

**Purpose:** Automated bug detection on code changes.

**Subcommands:**

### bugbot check

Run bugbot check on uncommitted changes.

```bash
tldr bugbot check

# Staged files only
tldr bugbot check --staged

# All uncommitted
tldr bugbot check --uncommitted
```

**Checks performed:**
- Syntax errors introduced
- Type errors from changes
- API contract violations
- Known bug patterns

---

## diff

**Alias:** `df`

**Purpose:** AST-aware structural diff between two files.

**Implementation:** `crates/tldr-cli/src/commands/diff.rs`

**Granularity levels:**
- `token` (L1) — Token-level diff
- `expression` (L2) — Expression-level diff
- `statement` (L3) — Statement-level diff
- `function` (L4) — Function-level diff (default)
- `class` (L5) — Class-level diff
- `file` (L6) — File-level diff
- `module` (L7) — Module-level diff
- `architecture` (L8) — Architecture-level diff

**Example:**
```bash
tldr diff src/v1/utils.py src/v2/utils.py

# Expression-level diff
tldr diff src/v1/main.py src/v2/main.py -g expression

# Exclude formatting-only changes
tldr diff src/v1/main.py src/v2/main.py --semantic-only
```

---

## surface

**Alias:** `surf`

**Purpose:** Extract machine-readable API surface for a library/package.

**Example:**
```bash
tldr surface requests

# Lookup specific API
tldr surface requests --lookup requests.Session

# Include private APIs
tldr surface mylib --include-private
```

---

## deps

**Alias:** `dep`

**Purpose:** Analyze module dependencies.

**Example:**
```bash
tldr deps src/

# Include external deps
tldr deps src/ --include-external

# Show cycles only
tldr deps src/ --show-cycles

# Limit depth
tldr deps src/ -d 3
```

---

## change-impact

**Alias:** `ci`

**Purpose:** Find tests affected by code changes.

**Example:**
```bash
tldr change-impact src/

# Explicit changed files
tldr change-impact src/ -F src/main.py,src/utils.py

# Base branch
tldr change-impact src/ -b origin/main

# pytest format
tldr change-impact src/ --runner pytest-k

# Jest format
tldr change-impact src/ --runner jest
```

---

## todo

**Purpose:** Aggregate improvement suggestions.

**Aggregates from:**
- `dead` — Dead code
- `complexity` — High complexity functions
- `cohesion` — Low cohesion classes
- `similar` — Similar code fragments

**Example:**
```bash
tldr todo src/

# Quick mode
tldr todo src/ --quick

# Specific detail
tldr todo src/ --detail dead_code
```
