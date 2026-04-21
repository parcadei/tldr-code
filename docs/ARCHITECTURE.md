# TLDR Architecture

**Version:** 2.0
**Purpose:** Token-efficient code analysis engine with 5-layer analysis stack

---

## Overview

TLDR (Token-efficient Language-agnostic Data Representation) is a Rust-based code analysis engine designed for:

- **Token Efficiency**: 95% token savings vs raw source code
- **5-Layer Analysis Stack**: AST → Call Graph → CFG → DFG → PDG
- **Multi-language Support**: 18 languages via tree-sitter
- **Fast Static Analysis**: No LSP required, syntactic analysis only

### Design Philosophy

```
Fast Static Analysis (Current)          Full Type Resolution (Not Done)
─────────────────────────              ─────────────────────────────
✓ Syntactic patterns                ✗ LSP integration
✓ Import resolution                  ✗ Type inference
✓ Call tracking                     ✗ Method resolution
✗ Method resolution (needs LSP)    ✗ Interface resolution
✗ Dynamic dispatch (needs LSP)     ✗ Complex type flows
```

TLDR intentionally avoids LSP integration to maintain speed and token efficiency.

---

## Crate Organization

```
tldr-code/
├── crates/
│   ├── tldr-core/        # Analysis engine
│   ├── tldr-cli/         # CLI application
│   ├── tldr-daemon/      # Background daemon
│   └── tldr-mcp/         # MCP server integration
├── docs/                 # Documentation
└── target/               # Build output
```

### tldr-core (`crates/tldr-core/`)

The core analysis engine. See individual modules below.

### tldr-cli (`crates/tldr-cli/`)

CLI interface built with `clap`. Commands are organized in `src/commands/`:

```
commands/
├── ast.rs          # tree, structure, extract, imports
├── callgraph.rs    # calls, impact, dead, refs
├── cfg.rs          # CFG extraction
├── dfg.rs          # DFG extraction
├── pdg.rs          # PDG and slicing
├── search.rs      # BM25 search
├── semantic.rs     # Embedding search
├── quality/        # smells, complexity, health
├── security/       # taint, vuln, api-check
├── patterns/       # design pattern detection
├── daemon/        # Daemon client (IPC)
└── ...
```

### tldr-daemon (`crates/tldr-daemon/`)

Background caching daemon using `axum` HTTP server:

- Unix socket on macOS/Linux
- TCP socket on Windows
- LRU cache for analysis results

### tldr-mcp (`crates/tldr-mcp/`)

MCP server exposing tools via JSON-RPC 2.0 over stdio:

```
tools/
├── ast.rs         # AST analysis tools
├── callgraph.rs   # Call graph tools
├── flow.rs        # Data flow tools
├── search.rs      # Search tools
├── quality.rs     # Quality tools
└── security.rs    # Security tools
```

---

## Analysis Layers

### Layer 1: AST (Abstract Syntax Tree)

**Purpose:** Parse source code, extract high-level structure

**Key files:**
- `tldr-core/src/ast/parser.rs` — Tree-sitter parser pool
- `tldr-core/src/ast/extract.rs` — Full module extraction
- `tldr-core/src/ast/imports.rs` — Import parsing

**Output types:**
```rust
pub struct ModuleInfo {
    pub file_path: PathBuf,
    pub language: Language,
    pub docstring: Option<String>,
    pub imports: Vec<ImportInfo>,
    pub functions: Vec<FunctionInfo>,
    pub classes: Vec<ClassInfo>,
    pub constants: Vec<FieldInfo>,
    pub call_graph: IntraFileCallGraph,
}
```

**CLI commands:** `tree`, `structure`, `extract`, `imports`

---

### Layer 2: Call Graph

**Purpose:** Build cross-file call relationships

**Key files:**
- `tldr-core/src/callgraph/builder.rs` — Main builder
- `tldr-core/src/callgraph/resolver.rs` — Module resolution
- `tldr-core/src/callgraph/languages/*.rs` — Per-language handlers

**Key types:**
```rust
pub struct ProjectCallGraph {
    pub edges: HashSet<CallEdge>,
    pub functions: HashMap<FunctionRef, FunctionMetadata>,
}

pub struct CallEdge {
    pub src_file: PathBuf,
    pub src_func: String,
    pub dst_file: PathBuf,
    pub dst_func: String,
    pub call_type: CallType,
    pub confidence: Confidence,
}
```

**CLI commands:** `calls`, `impact`, `dead`, `hubs`, `whatbreaks`, `refs`

---

### Layer 3: CFG (Control Flow Graph)

**Purpose:** Extract control flow within functions

**Key files:**
- `tldr-core/src/cfg/extractor.rs`

**Output types:**
```rust
pub struct CfgContext {
    pub entry: BlockId,
    pub blocks: HashMap<BlockId, CfgBlock>,
}

pub enum BlockType {
    Entry, Branch, LoopHeader, LoopBody, Return, Exit, Body,
}
```

---

### Layer 4: DFG (Data Flow Graph)

**Purpose:** Track variable definitions and uses

**Key files:**
- `tldr-core/src/dfg/extractor.rs`
- `tldr-core/src/dfg/reaching.rs`

**Output types:**
```rust
pub struct DfgContext {
    pub entry: BlockId,
    pub blocks: HashMap<BlockId, DfgBlock>,
}

pub enum RefType {
    Definition,  // x = value
    Update,      // x += value
    Use,         // f(x)
}
```

**CLI commands:** `reaching-defs`, `available`

---

### Layer 5: PDG (Program Dependence Graph)

**Purpose:** Combine control and data flow for slicing

**Key files:**
- `tldr-core/src/pdg/extractor.rs`
- `tldr-core/src/pdg/slice.rs`

**CLI commands:** `slice`, `chop`

---

## Data Flow Diagram

```
Source Code
    │
    ▼
┌─────────────────────────────────┐
│  Layer 1: AST                   │ → ModuleInfo
│  (ast/extract.rs)              │
└────────────┬────────────────────┘
             │
             ▼
┌─────────────────────────────────┐
│  Layer 2: Call Graph           │ → ProjectCallGraph
│  (callgraph/builder.rs)        │
└────────────┬────────────────────┘
             │
             ▼
┌─────────────────────────────────┐
│  Layer 3: CFG                   │ → CfgContext
│  (cfg/extractor.rs)             │
└────────────┬────────────────────┘
             │
             ▼
┌─────────────────────────────────┐
│  Layer 4: DFG                   │ → DfgContext
│  (dfg/extractor.rs)             │
└────────────┬────────────────────┘
             │
             ▼
┌─────────────────────────────────┐
│  Layer 5: PDG                   │ → PdgContext
│  (pdg/extractor.rs)             │
└────────────┬────────────────────┘
             │
             ▼
┌─────────────────────────────────┐
│  Program Slicing                │ → SliceResult
│  (pdg/slice.rs)                 │
└─────────────────────────────────┘
```

---

## Supported Languages

| Tier | Languages | Notes |
|------|-----------|-------|
| 1 | Python, Go, C, C++ | Most complete implementations |
| 2 | TypeScript, Rust, Ruby, Java | Full support |
| 3 | C#, Kotlin, Swift | Full support |
| 4 | Scala, PHP, Lua, Luau, Elixir, OCaml | Full support |

---

## Performance Targets

From `crates/tldr-cli/src/main.rs`:

- **Cold start**: <100ms (via lazy grammar loading)
- **Parse time**: <5ms per file
- **Call graph**: <5s for 10K LOC

---

## Output Formats

All commands support multiple formats via `--format`:

| Format | Use case |
|--------|----------|
| `json` | Structured output, machine consumption |
| `text` | Human-readable, colored output |
| `compact` | Minified JSON for piping |
| `sarif` | GitHub/VS Code integration |
| `dot` | Graphviz visualization |

---

## Caching Architecture

```
┌─────────────────┐     ┌─────────────────┐
│   tldr-cli      │────▶│   tldr-daemon   │
│                 │ IPC │   (background) │
└─────────────────┘     └────────┬────────┘
                                 │
                                 ▼
                        ┌─────────────────┐
                        │  LRU Cache      │
                        │  (memory)      │
                        └─────────────────┘
```

Cache key: `hash(tool_name + arguments + file_mtimes)`
Invalidation: Via `daemon notify` command
