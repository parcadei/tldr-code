# Metrics Commands

Metrics commands analyze code size, complexity, coverage, and similarity.

## coverage

**Alias:** `cov`

**Purpose:** Parse coverage reports (Cobertura XML, LCOV, coverage.py JSON).

**Implementation:** `crates/tldr-cli/src/commands/coverage.rs`

**Supported formats:**
- `cobertura` — GitLab/Jenkins standard
- `lcov` — llvm-cov, gcov
- `coveragepy` — coverage.py JSON

**Example:**
```bash
tldr coverage coverage.xml

# Per-file breakdown
tldr coverage coverage.xml --by-file

# Uncovered only
tldr coverage coverage.xml --uncovered

# Sort by coverage
tldr coverage coverage.xml --sort asc
```

---

## dice

**Purpose:** Compare similarity between two code fragments using Dice coefficient.

**Implementation:** `crates/tldr-cli/src/commands/dice.rs`

**Dice coefficient:** `(2 * |intersection|) / (|a| + |b|)`

**Example:**
```bash
# Compare two files
tldr dice src/utils.py src/helpers.py

# Compare specific functions
tldr dice src/utils.py::process src/helpers.py::process

# Compare line ranges
tldr dice src/utils.py:10:50 src/helpers.py:20:60

# No normalization
tldr dice src/utils.py src/helpers.py --normalize none
```

---

## similar

**Alias:** `sim`

**Purpose:** Find similar code fragments using embeddings.

**Example:**
```bash
tldr similar src/utils.py

# Specific function
tldr similar src/utils.py -F process_data

# Top 10 results
tldr similar src/utils.py -n 10
```

---

## definition

**Alias:** `def`

**Purpose:** Go-to-definition for symbols.

**Example:**
```bash
tldr definition src/main.py 10 5

# By symbol name
tldr definition --symbol process_data --file src/main.py
```

---

## explain

**Alias:** `exp`

**Purpose:** Comprehensive function analysis.

**Analysis includes:**
- Function signature (name, params, return type)
- Purity (side-effect analysis)
- Complexity metrics
- Callers (functions that call this)
- Callees (functions this calls)

**Example:**
```bash
tldr explain src/process.py process_data

# Deeper call graph
tldr explain src/process.py process_data --depth 5
```

---

## loc

**Purpose:** Count lines of code.

**Breakdown:**
- Code lines (executable)
- Comment lines
- Blank lines

**Example:**
```bash
tldr loc src/

# Per file
tldr loc src/ --by-file

# By directory
tldr loc src/ --by-dir
```

---

## cognitive

**Alias:** `cog`

**Purpose:** Calculate cognitive complexity.

**Example:**
```bash
tldr cognitive src/

# Per function
tldr cognitive src/ --function process_data

# With contributors
tldr cognitive src/ --function process_data --show-contributors
```

---

## halstead

**Alias:** `hal`

**Purpose:** Calculate Halstead metrics.

**Metrics:**
- **n1**: Unique operators
- **n2**: Unique operands
- **N1**: Total operators
- **N2**: Total operands
- **Volume**: N * log2(n)
- **Difficulty**: n1/2 * N2/n2
- **Effort**: Volume * Difficulty

**Example:**
```bash
tldr halstead src/process.py

# Show operators/operands lists
tldr halstead src/process.py --show-operators --show-operands
```

---

## hotspots

**Alias:** `hot`

**Purpose:** Find churn x complexity hotspots.

**Example:**
```bash
tldr hotspots src/

# Function level
tldr hotspots src/ --by-function

# Different time window
tldr hotspots src/ --days 180
```
