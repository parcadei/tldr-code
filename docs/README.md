# TLDR Documentation

Token-efficient code analysis for LLMs with 60+ commands across AST, call graph, data flow, security, and quality analysis.

## Contents

### Getting Started
- [Installation](INSTALL.md) — Binary releases, source build, dependencies
- [Setup Guide](SETUP.md) — Initial configuration, language setup, first steps
- [Troubleshooting](TROUBLESHOOTING.md) — Common issues and solutions

### Integration
- [MCP Server](MCP.md) — Model Context Protocol integration for Claude Code and other MCP clients

### Reference
- [Architecture](ARCHITECTURE.md) — System design, analysis layers, crate organization
- [Commands](commands/) — Detailed command reference

### Command Categories

| Category | Layer | Commands |
|----------|-------|----------|
| [AST Analysis](commands/ast.md) | L1 | `tree`, `structure`, `extract`, `imports`, `importers` |
| [Call Graph](commands/callgraph.md) | L2 | `calls`, `impact`, `dead`, `hubs`, `whatbreaks`, `refs` |
| [Data Flow](commands/dataflow.md) | L3-L4 | `reaching-defs`, `available`, `dead-stores`, `slice`, `chop` |
| [Search](commands/search.md) | — | `search`, `semantic`, `similar`, `context` |
| [Quality](commands/quality.md) | — | `smells`, `complexity`, `cognitive`, `halstead`, `loc`, `churn`, `debt`, `health`, `hotspots`, `clones`, `cohesion`, `coupling` |
| [Security](commands/security.md) | — | `taint`, `vuln`, `api-check`, `secure`, `resources` |
| [Patterns](commands/patterns.md) | — | `patterns`, `inheritance`, `contracts`, `specs`, `invariants`, `verify`, `temporal`, `interface` |
| [Metrics](commands/metrics.md) | — | `coverage`, `dice`, `similar`, `definition`, `explain` |
| [Daemons](commands/daemon.md) | — | `daemon`, `cache`, `warm`, `stats` |
| [Tools](commands/tools.md) | — | `doctor`, `diagnostics`, `fix`, `bugbot`, `diff`, `surface`, `deps`, `change-impact`, `todo` |

---

## Quick Start

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

## Output Formats

All commands support multiple output formats via `--format`:

```bash
--format json      # Structured JSON (default)
--format text      # Human-readable
--format compact  # Minified JSON
--format sarif    # GitHub/VS Code integration
--format dot      # Graphviz visualization
```

## Architecture

TLDR uses a **5-layer analysis stack** for token-efficient code understanding:

```
Layer 1: AST      → File structure, functions, classes
Layer 2: CallGraph → Cross-file call relationships
Layer 3: CFG      → Control flow within functions
Layer 4: DFG      → Data flow (variable definitions/uses)
Layer 5: PDG      → Program dependence for slicing
```

Each higher layer builds on the ones below it, enabling analyses like:
- **Impact analysis** (L2): Who calls this function?
- **Taint tracking** (L4+L5): Does user input reach this SQL query?
- **Program slicing** (L5): What code affects this line?
