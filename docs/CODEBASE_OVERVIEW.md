# Codebase Overview

This document summarizes what `tldr-code` contains, based on local analysis with
the `tldr` CLI and focused source inspection.

## What this project is

`tldr-code` is a Rust code-intelligence toolkit designed to give compact,
LLM-friendly answers about source code. It is exposed through three main entry
points:

- `tldr-cli`: command-line interface and user-facing command routing.
- `tldr-daemon`: background server for cached analysis requests.
- `tldr-mcp`: MCP protocol server exposing analysis tools to agents.
- `tldr-core`: reusable analysis engine shared by the other crates.

The core idea is to parse code, build increasingly rich program-analysis models,
and return only the context needed for a task.

## Size and language mix

`tldr loc crates` reported:

- **974 files** under `crates/`
- **577,658 total lines**
- **423,731 code lines**
- **91,539 comment lines**
- **62,388 blank lines**

Rust is the dominant implementation language:

- **736 Rust files**
- **421,966 Rust code lines**
- **574,988 total Rust lines**

The repository also contains small source files in C, C++, C#, Elixir, Go, Java,
JavaScript, Kotlin, Lua, Luau, OCaml, PHP, Python, Ruby, Scala, Swift, and
TypeScript. Most of these are fixtures used to validate multi-language parsing
and command behavior.

## Main architecture

The architecture is intentionally layered:

```text
CLI / MCP / daemon entry points
        |
        v
Shared command and request handling
        |
        v
tldr-core analysis engine
        |
        +-- AST / structure extraction
        +-- call graph, import graph, impact, dead code
        +-- CFG and complexity
        +-- DFG, reaching definitions, dataflow
        +-- PDG, slicing, dependency paths
        +-- SSA, dominators, live variables, SCCP
        +-- search, similarity, clone detection
        +-- security, secrets, taint, vulnerability checks
        +-- quality, smells, maintainability, coverage parsing
```

The layered design is visible in `tldr-core/src/lib.rs`, where the code is
organized around analysis layers such as call graph, CFG, DFG, PDG, security,
quality, search, and context building.

## Important subsystems

### Core analysis engine

`tldr-core` owns the reusable logic. It handles language detection, file walking,
Tree-sitter parsing, structure extraction, graph construction, dataflow,
security checks, quality metrics, and search.

### CLI

`tldr-cli` owns command definitions, argument parsing, output formatting, and
integration with the daemon. It supports multiple output formats including text,
JSON, DOT, and SARIF for selected commands.

### Daemon

`tldr-daemon` keeps analysis state alive between commands. It caches expensive
queries and serves requests over a local IPC/socket path. Recent debugging added
persistent daemon logging to `.tldr/daemon.log`.

### MCP server

`tldr-mcp` exposes the same analysis ideas as MCP tools so external agents can
query code structure, call graphs, flows, search, security, and quality reports.

## Libraries used

Notable dependencies include:

- **Parsing:** `tree-sitter` plus language grammars for Python, TypeScript, Go,
  Rust, Java, C, C++, Ruby, Kotlin, Swift, C#, Scala, PHP, Lua, Luau, Elixir,
  and OCaml.
- **CLI and output:** `clap`, `colored`, `comfy-table`, `serde`, `serde_json`,
  `serde_yaml`.
- **Async/server:** `tokio`, `axum`, `tower`, `hyper`, `hyper-util`.
- **Walking and paths:** `ignore`, `walkdir`, `dunce`, `dirs`, `glob`.
- **Performance/data structures:** `rayon`, `lru`, `bitvec`, `rustc-hash`,
  `hashbrown`, `smallvec`, `bumpalo`, `typed-arena`, `radix-heap`.
- **Search/similarity:** `regex`, `strsim`, optional `fastembed` for semantic
  search.
- **Testing:** `assert_cmd`, `predicates`, `proptest`, `criterion`, `tempfile`,
  `serial_test`.

## Manual algorithms and analyses

The project implements many algorithms directly rather than only wrapping third
party tools:

- Control-flow graph construction.
- Data-flow graph construction.
- Program-dependence graph construction.
- Static single assignment form.
- Dominator tree and dominance-frontier support.
- Live-variable analysis.
- Reaching definitions.
- Available expressions.
- Sparse conditional constant propagation style SSA analysis.
- Memory SSA support.
- CFG-based taint analysis.
- Vulnerability pattern detection.
- Secrets scanning.
- BM25 search and hybrid search plumbing.
- Reciprocal-rank fusion style result combination.
- Clone and similarity detection.
- Cyclomatic, cognitive, Halstead, and maintainability metrics.
- Change-impact and dead-code analysis.
- Abstract interpretation and octagon-domain components.

This is a meaningful amount of compiler/static-analysis work.

## Test suite quality

The test suite is large and broad. A focused scan found approximately:

- **12,546 Rust test attributes** (`#[test]`, `#[tokio::test]`, and property
  test declarations)
- **515 test-related files**
- **16 benchmark or Criterion-related files**

Coverage appears to span:

- CLI command behavior and output formatting.
- Exhaustive command/language matrices.
- Language autodetection.
- Tree-sitter extraction fixtures across many languages.
- CFG, DFG, PDG, SSA, dominators, taint, and security behavior.
- Search and enriched search behavior.
- Daemon and cache behavior.
- MCP lifecycle and request formatting.
- Property tests and performance benchmarks.

The test quality looks strong for breadth and regression protection. The main
caveat is that a very large generated/matrix-style suite can hide whether each
individual analysis has deep semantic assertions. Still, this repository clearly
has more test investment than a typical CLI project.

## Architecture assessment

The codebase shows substantial architectural thought:

- Clear separation between core analysis, CLI, daemon, and MCP.
- Layered program-analysis concepts rather than ad-hoc grep wrappers.
- Multi-language support with fixtures and command matrices.
- Multiple machine-readable output formats.
- Daemon caching and warm-cache design for repeated expensive operations.
- Security, quality, complexity, coverage, and search treated as first-class
  subsystems.

The main engineering risk is complexity. The project has many analyses, command
paths, caches, and output formats. That gives it power, but it also creates
performance and maintenance hotspots. The earlier daemon investigation showed an
example: search itself can be fast, while client-side callgraph enrichment can
make the full command slow.

Overall, this is a mature and ambitious static-analysis/code-intelligence
project with real compiler-analysis ideas, strong testing investment, and a
deliberate multi-interface architecture.
