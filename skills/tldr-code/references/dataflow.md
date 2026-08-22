# Data Flow Commands (Layers 3-5)

Data flow commands track how values move through code.

## reaching-defs

**Alias:** `rd`

**Purpose:** Analyze reaching definitions for a function.

**Implementation:** `crates/tldr-cli/src/commands/reaching_defs.rs`

```rust
// Reaching definitions analysis
pub struct ReachingDefsArgs {
    pub file: PathBuf,
    pub function: String,
    pub var: Option<String>,
    pub line: Option<u32>,
    pub show_chains: bool,
    pub show_uninitialized: bool,
    pub show_in_out: bool,
}
```

**How it works:**
1. Builds CFG for function
2. Computes IN/OUT sets per block using dataflow framework
3. Tracks where each variable definition reaches

**Example:**
```bash
tldr reaching-defs src/process.py process_data

# Filter by variable
tldr reaching-defs src/process.py process_data --var user_input

# Show at specific line
tldr reaching-defs src/process.py process_data --line 25

# Show def-use chains
tldr reaching-defs src/process.py process_data --show-chains
```

**Output:**
```json
{
  "function": "process_data",
  "blocks": [...],
  "def_use_chains": [
    {
      "variable": "result",
      "definition": {"line": 10, "block": 1},
      "uses": [{"line": 15}, {"line": 20}]
    }
  ]
}
```

---

## available

**Alias:** `av`

**Purpose:** Analyze available expressions for CSE (Common Subexpression Elimination).

**Implementation:** `crates/tldr-cli/src/commands/available.rs`

**How it works:**
1. Builds CFG for function
2. Computes available expressions per block
3. An expression is "available" if all paths to a point have computed it
4. Identifies CSE opportunities

**Example:**
```bash
tldr available src/process.py process_data

# Check specific expression
tldr available src/process.py process_data --check "a + b"

# At specific line
tldr available src/process.py process_data --at-line 50

# Show what kills an expression
tldr available src/process.py process_data --killed-by "x + y"
```

---

## dead-stores

**Alias:** `ds`

**Purpose:** Find dead stores using SSA-based analysis.

**Implementation:** `crates/tldr-cli/src/commands/contracts/dead_stores.rs`

**How it works:**
1. Converts function to SSA form
2. Identifies assignments that are never read
3. Returns lines where value is stored but never used

**Example:**
```bash
tldr dead-stores src/process.py process_data

# Compare with live-variables approach
tldr dead-stores src/process.py process_data --compare
```

---

## slice

**Purpose:** Compute program slice (backward or forward).

**Implementation:** `crates/tldr-cli/src/commands/slice.rs`

```rust
// Program slicing
pub struct SliceArgs {
    pub file: PathBuf,
    pub function: String,
    pub line: u32,
    pub direction: Direction,  // backward or forward
    pub variable: Option<String>,
}
```

**How it works:**
1. Builds PDG (Program Dependence Graph)
2. **Backward slice**: All statements affecting this line
3. **Forward slice**: All statements affected by this line
4. Optionally filter by variable

**Example:**
```bash
tldr slice src/process.py process_data 25

# Forward slice
tldr slice src/process.py process_data 25 -d forward

# Filter by variable
tldr slice src/process.py process_data 25 --variable result
```

---

## chop

**Alias:** `chp`

**Purpose:** Compute chop slice — intersection of forward and backward slices.

**How it works:**
1. Computes backward slice from source
2. Computes forward slice from target
3. Returns intersection: statements that affect target AND are affected by source

**Example:**
```bash
# Statements from line 10 that affect line 50
tldr chop src/process.py process_data 10 50
```
