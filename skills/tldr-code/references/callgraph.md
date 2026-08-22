# Call Graph Commands (Layer 2)

Layer 2 commands analyze relationships between functions across files.

## calls

**Alias:** `c`

**Purpose:** Build cross-file call graph.

**Implementation:** `crates/tldr-cli/src/commands/calls.rs`

```rust
// From calls.rs
pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
    let graph = build_project_call_graph(
        &self.path,
        self.language,
        None,
        self.respect_ignore,
    )?;
}
```

**How it works:**
1. Walks directory finding all source files
2. Parses each file with tree-sitter
3. Extracts function definitions
4. Resolves import statements to file paths
5. Builds edges: caller → callee relationships

**Example:**
```bash
# Build call graph
tldr calls src/

# Respect .gitignore
tldr calls src/ --respect-ignore

# Limit edges
tldr calls src/ --max-items 100
```

**Output structure:**
```json
{
  "edges": [
    {
      "src_file": "src/main.py",
      "src_func": "main",
      "dst_file": "src/utils.py",
      "dst_func": "process",
      "call_type": "Direct"
    }
  ],
  "functions": {
    "src/main.py::main": {
      "line": 5,
      "is_public": true
    }
  }
}
```

---

## impact

**Alias:** `i`

**Purpose:** Analyze impact of changing a function — who calls it?

**Implementation:** `crates/tldr-cli/src/commands/impact.rs`

```rust
// From impact.rs
pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
    let report = impact_analysis_with_ast_fallback(
        &self.path,
        &self.function,
        self.file.as_deref(),
        self.language,
        self.depth,
    )?;
}
```

**How it works:**
1. Takes function name + optional file filter
2. Builds reverse call graph (who → target)
3. Traverses up to `--depth` levels
4. Returns all functions that transitively call target

**Example:**
```bash
# Who calls parse_config?
tldr impact parse_config src/

# With depth limit
tldr impact parse_config src/ -d 3

# Type-aware resolution
tldr impact process_data src/ --type-aware
```

---

## dead

**Alias:** `d`

**Purpose:** Find dead (unreachable) code.

**Implementation:** `crates/tldr-cli/src/commands/dead.rs`

```rust
// Two analysis modes (dead.rs:111-145)
if self.call_graph {
    // Mode 1: Call graph based (slower, more accurate)
    dead_code_analysis(&graph, &all_functions, entry_points)
} else {
    // Mode 2: Reference counting (faster, single-pass)
    dead_code_analysis_refcount(&all_functions, &merged_ref_counts, entry_points)
}
```

**How it works:**

1. **Reference counting mode (default):**
   - Counts identifier occurrences per file
   - Functions with count=1 (only self-reference) are dead
   - Fast: single pass through AST

2. **Call graph mode:**
   - Builds full call graph
   - Marks entry points (main, public API)
   - Traverses call graph to find unreachable
   - More accurate but slower

**Example:**
```bash
# Default (reference counting)
tldr dead src/

# Call graph mode
tldr dead src/ --call-graph

# With custom entry points
tldr dead src/ -e main,api_v1,WebHandler
```

**Output:**
```json
{
  "total_functions": 150,
  "total_dead": 12,
  "dead_percentage": 8.0,
  "by_file": {
    "src/utils.py": ["unused_helper", "old_format"]
  }
}
```

---

## hubs

**Purpose:** Detect hub functions using centrality analysis.

**Implementation:** `crates/tldr-cli/src/commands/hubs.rs`

**How it works:**
1. Builds call graph from all files
2. Computes centrality metrics:
   - **In-degree**: Many functions call this
   - **Out-degree**: This calls many functions
   - **PageRank**: Important in call graph
   - **Betweenness**: Bridge between modules
3. Returns top N hub functions

**Example:**
```bash
tldr hubs src/

# Algorithm selection
tldr hubs src/ --algorithm pagerank

# Top 20
tldr hubs src/ --top 20
```

---

## whatbreaks

**Alias:** `wb`

**Purpose:** Analyze what breaks if a target is changed.

**Implementation:** `crates/tldr-cli/src/commands/whatbreaks.rs`

**How it works:**
1. Accepts target (function, file, or module)
2. Auto-detects target type
3. Runs appropriate analysis:
   - Function → impact analysis
   - File → importers + change-impact
   - Module → importers

**Example:**
```bash
tldr whatbreaks src/utils.py

# Force function type
tldr whatbreaks process_data -t function src/
```

---

## references

**Alias:** `refs`

**Purpose:** Find all references to a symbol.

**How it works:**
1. Builds cross-file reference index
2. Searches for identifier occurrences
3. Filters by reference kind

**Example:**
```bash
tldr references my_function src/ -t call,read
```
