# Setup Guide

## Initial Configuration

After [installing](INSTALL.md) TLDR, follow this guide to get started.

## First Run

```bash
tldr --version
tldr doctor
```

The `doctor` command checks your environment and can install missing diagnostic tools.

## Configuration Files

TLDR respects the following configuration:

### Project-level `.tldrignore`

Place in project root to exclude files/directories:

```
# Generated files
*.generated.ts
*_pb.go

# Vendor directories
vendor/
node_modules/

# Test files (for certain analyses)
*_test.py
*_spec.rb
```

### Daemon Configuration

The daemon uses a Unix socket by default (macOS/Linux) or TCP port (Windows).

Location: `~/.cache/tldr/` or project-specific `.tldr/` directory.

```bash
# Check daemon status
tldr daemon status

# Start daemon (for repeated queries)
tldr daemon start

# Stop daemon
tldr daemon stop
```

## Language Detection

TLDR auto-detects language from file extension:

```bash
# Explicit language
tldr structure src/ -l python

# Auto-detected
tldr structure src/   # Detects from .py, .rs, .go, etc.
```

### Supported Languages

| Language | Extensions | Notes |
|----------|------------|-------|
| Python | `.py`, `.pyi` | Full support |
| TypeScript | `.ts`, `.tsx` | Full support |
| JavaScript | `.js`, `.jsx`, `.mjs` | Uses TS grammar |
| Go | `.go` | Full support |
| Rust | `.rs` | Full support |
| Java | `.java` | Full support |
| C | `.c`, `.h` | Full support |
| C++ | `.cpp`, `.cc`, `.cxx`, `.hpp` | Full support |
| Ruby | `.rb` | Full support |
| Kotlin | `.kt`, `.kts` | Full support |
| Swift | `.swift` | Full support |
| C# | `.cs` | Full support |
| Scala | `.scala` | Full support |
| PHP | `.php` | Full support |
| Lua | `.lua` | Full support |
| Luau | `.luau` | Full support |
| Elixir | `.ex`, `.exs` | Full support |
| OCaml | `.ml`, `.mli` | Full support |

## Quick Tutorial

### 1. Explore a Codebase

```bash
# See file structure
tldr tree src/

# Get code structure
tldr structure src/

# Search for a function
tldr search parse_config src/
```

### 2. Understand Code Relationships

```bash
# Build call graph
tldr calls src/

# Find who calls a function
tldr impact process_data src/

# Find dead code
tldr dead src/
```

### 3. Analyze Quality

```bash
# Get health dashboard
tldr health src/

# Check for code smells
tldr smells src/

# Find hotspots (churn x complexity)
tldr hotspots src/
```

### 4. Security Analysis

```bash
# Full security dashboard
tldr secure src/

# Taint analysis on specific function
tldr taint src/process.py process_user_input

# Vulnerability scan
tldr vuln src/
```

## Daemon Mode

For repeated analysis of the same codebase, use the daemon:

```bash
# Start daemon
tldr daemon start

# Pre-warm cache
tldr warm src/

# Now queries are ~35x faster (cached)
tldr calls src/      # Fast - cache hit
tldr impact foo src/  # Fast - cache hit

# Stop daemon
tldr daemon stop
```

### Why Use Daemon?

- **Speed**: 35x faster for repeated queries via caching
- **Memory**: Analysis results stay in memory
- **Persistence**: Pre-warm cache survives between sessions

## Output Formats

### JSON (Default)

Machine-readable, consistent field ordering:

```bash
tldr structure src/ -f json
```

### Text (Human-readable)

Colored, formatted output:

```bash
tldr structure src/ -f text
```

### SARIF (CI/IDE Integration)

GitHub Code Scanning, VS Code compatible:

```bash
tldr smells src/ -f sarif > results.sarif
```

### DOT (Graphviz)

Visualize graphs:

```bash
tldr calls src/ -f dot > graph.dot
dot -Tpng graph.dot > graph.png
```

## Performance Tips

### Large Codebases

For projects with 10,000+ files:

```bash
# Increase file limits
tldr dead src/ --max-items 1000

# Use daemon for repeated queries
tldr daemon start
tldr warm src/  # Background warming
```

### Memory Management

```bash
# Check cache stats
tldr cache stats

# Clear old cache
tldr cache clear

# Memory limit for daemon (seconds idle before shutdown)
# Default: 300 seconds
```

## IDE Integration

### VS Code

Use the SARIF output format with VS Code's GitHub Security extension:

```bash
tldr vuln src/ -f sarif > vulns.sarif
```

### GitHub Actions

```bash
# Run analysis and upload as SARIF
- name: Run TLDR analysis
  run: |
    tldr secure src/ -f sarif > results.sarif
- name: Upload results
  uses: github/codeql-action/upload-sarif@v2
  with:
    sarif_file: results.sarif
```

## Next Steps

- Read the [Architecture](ARCHITECTURE.md) document to understand how TLDR works
- Browse [Command Reference](commands/) for detailed usage of each command
- Configure [MCP integration](MCP.md) for Claude Code
