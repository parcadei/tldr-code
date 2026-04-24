# tldr

Token-efficient code analysis for LLMs. 40+ commands across AST, call graph, data flow, security, and quality — output optimized for machine consumption.

## Why

LLMs waste context on raw source dumps. tldr extracts the signal: function signatures, call graphs, taint flows, complexity metrics, dead code — as structured JSON that fits in a fraction of the tokens.

**18 languages**: Python, TypeScript, JavaScript, Go, Rust, Java, C, C++, Ruby, Kotlin, Swift, C#, Scala, PHP, Lua, Luau, Elixir, OCaml.

## Install

### Binary (recommended)

Download the latest binary for macOS or Linux from [GitHub Releases](https://github.com/parcadei/tldr-code/releases). Windows users should install via cargo (see below).

> **Note:** Pre-built binaries do not include semantic search (`tldr semantic`, `tldr similar`). For those commands, install via cargo with `--features semantic` (see below).

### Cargo

```bash
cargo install tldr-cli
```

With semantic search (natural language code search via embeddings):

```bash
cargo install tldr-cli --features semantic
```

## Quick start

```bash
# What's in this codebase?
tldr structure src/

# Who calls this function?
tldr impact parse_config src/

# Find dead code
tldr dead src/

# Security scan
tldr secure src/

# Full health dashboard
tldr health src/
```

## Commands

### AST Analysis (L1)
| Command | Description |
|---------|-------------|
| `tree` | File tree structure |
| `structure` | Code structure — functions, classes, imports |
| `extract` | Complete module info |
| `imports` | Parse import statements |
| `importers` | Find files importing a module |

### Call Graph (L2)
| Command | Description |
|---------|-------------|
| `calls` | Cross-file call graph |
| `impact` | Reverse call graph — who calls this? |
| `dead` | Dead code detection |
| `hubs` | Hub functions (centrality analysis) |
| `whatbreaks` | What breaks if target changes? |

### Data Flow (L3-L4)
| Command | Description |
|---------|-------------|
| `reaching-defs` | Reaching definitions |
| `available` | Available expressions (CSE detection) |
| `dead-stores` | Dead store detection (SSA-based) |

### Program Dependence (L5)
| Command | Description |
|---------|-------------|
| `slice` | Backward program slice |
| `chop` | Chop slice (forward + backward intersection) |
| `taint` | Taint flow analysis |

### Security
| Command | Description |
|---------|-------------|
| `secure` | Security dashboard |
| `taint` | Taint flows (injection, XSS) |
| `vuln` | Vulnerability scanning |
| `api-check` | API misuse patterns |
| `resources` | Resource leak detection |

### Quality & Metrics
| Command | Description |
|---------|-------------|
| `smells` | Code smells |
| `complexity` | Cyclomatic complexity |
| `cognitive` | Cognitive complexity |
| `halstead` | Halstead metrics |
| `loc` | Lines of code |
| `churn` | Git churn analysis |
| `debt` | Technical debt (SQALE) |
| `health` | Health dashboard |
| `hotspots` | Churn x complexity |
| `clones` | Code clone detection |
| `cohesion` | LCOM4 cohesion |
| `coupling` | Afferent/efferent coupling |

### Patterns & Architecture
| Command | Description |
|---------|-------------|
| `patterns` | Design pattern detection |
| `inheritance` | Class hierarchies |
| `surface` | API surface extraction |

### Contracts & Verification
| Command | Description |
|---------|-------------|
| `contracts` | Pre/postcondition inference |
| `specs` | Extract test specs |
| `invariants` | Infer invariants from tests |
| `verify` | Verification dashboard |
| `interface` | Interface contracts |

### Search & Context
| Command | Description |
|---------|-------------|
| `search` | BM25 search with structural context |
| `semantic` | Natural language code search * |
| `similar` | Find similar code fragments * |
| `context` | LLM-ready context from entry point |
| `definition` | Go-to-definition |
| `explain` | Comprehensive function analysis |

\* Requires the `semantic` feature: `cargo install tldr-cli --features semantic`

### Aggregated
| Command | Description |
|---------|-------------|
| `todo` | Improvement suggestions |
| `diff` | AST-aware structural diff |
| `fix` | Diagnose and auto-fix errors |
| `bugbot` | Automated bug detection on changes |

## Output formats

```bash
--format json      # Default — structured, machine-readable
--format text      # Human-readable
--format compact   # Minified JSON for piping
--format sarif     # GitHub/VS Code integration
--format dot       # Graphviz visualization
```

## Daemon mode

For repeated queries, the daemon caches results in memory:

```bash
tldr daemon start
tldr warm src/          # Pre-warm cache
tldr calls src/         # Fast — cache hit
tldr daemon stop
```

## Documentation

For detailed documentation, see the [docs/](docs/) folder:
- [Installation Guide](docs/INSTALL.md)
- [Setup Guide](docs/SETUP.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [MCP Integration](docs/MCP.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Command Reference](docs/commands/)

## License

AGPL-3.0
