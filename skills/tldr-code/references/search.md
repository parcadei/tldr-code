# Search Commands

Search commands find code by content or semantic similarity.

## search

**Purpose:** Enriched search with function-level context cards (BM25 + structure + call graph).

**Implementation:** `crates/tldr-cli/src/commands/search.rs`

```rust
pub struct SmartSearchArgs {
    pub query: String,
    pub path: PathBuf,
    pub top_k: usize,
    pub no_callgraph: bool,
    pub regex: bool,
    pub hybrid: Option<String>,
}
```

**How it works:**
1. **BM25 ranking**: Text search with TF-IDF weighting
2. **Structural context**: Enriches results with function signatures
3. **Call graph**: Adds callers/callees to result cards
4. **Hybrid mode**: Combine BM25 + regex filtering

**Example:**
```bash
tldr search "parse config" src/

# Return top 5
tldr search "error handling" src/ -k 5

# Skip call graph (faster)
tldr search "validate" src/ --no-callgraph

# Regex mode
tldr search "get.*user" src/ --regex

# Hybrid: BM25 ranking with regex filtering
tldr search "handler" src/ --hybrid ".*_handler"
```

**Output:**
```json
{
  "results": [
    {
      "file": "src/handlers.py",
      "function": "handle_user_request",
      "line": 42,
      "snippet": "def handle_user_request(config):",
      "score": 0.85,
      "callers": ["main", "router"],
      "callees": ["validate", "process"]
    }
  ]
}
```

---

## semantic

**Alias:** `sem`

**Purpose:** Semantic code search using natural language.

**Implementation:** `crates/tldr-cli/src/commands/semantic.rs`

```rust
pub struct SemanticArgs {
    pub query: String,
    pub path: PathBuf,
    pub top: usize,
    pub threshold: f32,
    pub model: String,
    pub lang: Option<Language>,
    pub no_cache: bool,
}
```

**How it works:**
1. Embeds query using FastEmbed model (arctic-s/m/l)
2. Embeds code chunks (function-level granularity)
3. Computes cosine similarity
4. Returns top N semantically similar results

**Example:**
```bash
tldr semantic "how is user authentication handled" src/

# Custom threshold
tldr semantic "session management" src/ -t 0.7

# Top 5
tldr semantic "database queries" src/ -n 5

# Different model
tldr semantic "caching" src/ -m arctic-l
```

---

## similar

**Alias:** `sim`

**Purpose:** Find similar code fragments to a given file/function.

**Implementation:** `crates/tldr-cli/src/commands/similar.rs`

**How it works:**
1. Embeds target function/file
2. Compares against all functions in scope
3. Returns ranked list of similar code

**Example:**
```bash
tldr similar src/utils.py

# Specific function
tldr similar src/utils.py -F process_data

# Different search path
tldr similar src/utils.py -p src/

# No cache
tldr similar src/utils.py --no-cache
```

---

## context

**Purpose:** Build LLM-ready context from entry point.

**Implementation:** `crates/tldr-cli/src/commands/context.rs`

```rust
pub struct ContextArgs {
    pub entry: String,
    pub project: PathBuf,
    pub depth: usize,
    pub include_docstrings: bool,
    pub file: Option<String>,
}
```

**How it works:**
1. Starts from entry function
2. Recursively collects:
   - Function signature
   - Docstring
   - Local context (variables, helpers)
   - Called functions (up to depth N)
3. Formats for LLM consumption (token-efficient)

**Example:**
```bash
tldr context main src/

# Deeper context
tldr context main src/ -d 5

# Include docstrings
tldr context main src/ --include-docstrings

# Specific file disambiguation
tldr context render src/ --file src/renderer.py
```

**Output:**
```
=== Function: main ===
def main() -> None:
    processes user input

=== Callee: parse_input (line 10) ===
def parse_input(data: str) -> dict:
    validates and parses input

=== Callee: process (line 25) ===
def process(config: dict) -> None:
    ...
```
