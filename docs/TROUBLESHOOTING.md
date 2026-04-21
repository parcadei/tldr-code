# Troubleshooting

Common issues and solutions for TLDR.

## Installation Issues

### "Command not found" after installing binary

1. Verify the binary is in your PATH:
```bash
which tldr
# or
echo $PATH | tr ':' '\n' | xargs -I{} ls -la {}/tldr 2>/dev/null
```

2. If using `~/.local/bin/`, ensure it's in your PATH:
```bash
export PATH="$HOME/.local/bin:$PATH"
```

3. Add to shell profile (`~/.zshrc`, `~/.bashrc`):
```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

### macOS: "tldr cannot be opened because it is from an unidentified developer"

1. Go to System Preferences > Security & Privacy > General
2. Click "Open Anyway" next to the TLDR message
3. Or run: `xattr -d com.apple.quarantine $(which tldr)`

### Rust: "linker cc not found" when building

Install the C compiler:

```bash
# macOS
xcode-select --install

# Ubuntu/Debian
sudo apt install build-essential

# Fedora/RHEL
sudo dnf install gcc
```

## Runtime Errors

### "Unsupported language" for valid file

TLDR auto-detects language from file extension. If detection fails:

```bash
# Specify language explicitly
tldr structure src/ -l python
tldr calls src/ -l go
```

Valid language values: `python`, `typescript`, `javascript`, `go`, `rust`, `java`, `c`, `cpp`, `ruby`, `kotlin`, `swift`, `csharp`, `scala`, `php`, `lua`, `luau`, `elixir`, `ocaml`

### "Parse error" or "Failed to parse file"

This usually means the tree-sitter grammar version doesn't match the source code syntax:

1. Check your language version:
```bash
# For Python
python3 --version
```

2. Check TLDR's grammar version:
```bash
grep tree-sitter-python Cargo.lock
```

3. If versions mismatch, try [building from source](INSTALL.md) with correct grammar versions.

### Slow performance on large codebases

1. Use the **daemon** for caching:
```bash
tldr daemon start
tldr warm src/
tldr calls src/  # Now cached
```

2. Limit analysis scope:
```bash
tldr structure src/ --max-results 100
tldr dead src/ --max-items 50
```

3. Exclude unnecessary directories:
```bash
# Create .tldrignore
echo "vendor/" >> .tldrignore
echo "node_modules/" >> .tldrignore
```

### Out of memory on large codebase

1. Limit concurrent file processing:
```bash
# Process fewer files at once
find src/ -name "*.py" | head -1000 | xargs tldr structure
```

2. Use daemon with smaller cache:
```bash
# Daemon auto-shuts down after 300s idle
# Adjust idle timeout (in seconds)
tldr daemon start --idle-timeout 600
```

3. Use shallow analysis:
```bash
tldr dead src/ --max-items 100
```

## Daemon Issues

### "Daemon not running" or connection errors

1. Check daemon status:
```bash
tldr daemon status
```

2. Start the daemon:
```bash
tldr daemon start
```

3. If still failing, remove stale socket and restart:
```bash
rm -f ~/.cache/tldr/*.sock
tldr daemon start
```

### Daemon uses too much memory

The daemon caches analysis results. For very large codebases:

1. Clear cache periodically:
```bash
tldr cache clear
```

2. Use `--max-items` limits:
```bash
tldr warm src/ --max-items 1000
```

3. Stop daemon when not needed:
```bash
tldr daemon stop
```

### macOS: "Permission denied" on socket

```bash
# Remove stale socket
rm -rf ~/.cache/tldr/

# Restart daemon
tldr daemon start
```

## Output Issues

### JSON output is not valid

TLDR uses `preserve_order` in serde_json for consistent output. If you see issues:

1. Check JSON validity:
```bash
tldr structure src/ | jq .
```

2. Try compact format:
```bash
tldr structure src/ -f compact
```

3. Report issue at [GitHub](https://github.com/parcadei/tldr-code/issues)

### Text output shows garbled characters

Text output uses ANSI colors. If your terminal doesn't support colors:

```bash
# Force no colors (or use --no-color)
NO_COLOR=1 tldr structure src/ -f text
```

### SARIF output not accepted by GitHub

1. Validate SARIF format:
```bash
tldr vuln src/ -f sarif > results.sarif
# Upload to https://github.com/tools/sarif-validator
```

2. Check GitHub upload size limits (max 10MB SARIF)

3. Filter to high-severity only:
```bash
tldr vuln src/ --severity high -f sarif > results.sarif
```

## Analysis Issues

### False positives in dead code detection

TLDR uses reference counting by default (faster, lower memory):

```bash
# Use call graph analysis (more accurate but slower)
tldr dead src/ --call-graph
```

For Next.js/React "use server" directives, dead code is tagged to avoid false positives:

```bash
# Functions with 'use server' are excluded from dead code
# Unless they're truly unreachable
```

### Taint analysis misses flows

Taint analysis isintra-file by default (within a single function):

```bash
# For cross-file taint tracking, use:
tldr secure src/  # Full security dashboard
```

### "File not found" for files that exist

1. Check relative vs absolute paths:
```bash
# Relative to current directory
tldr structure ./src/main.py

# Absolute path
tldr structure /path/to/project/src/main.py
```

2. Check file permissions:
```bash
ls -la src/main.py
```

## MCP Server Issues

### MCP server not starting

1. Check configuration in Claude Code:
```bash
# In Claude Code settings, check MCP servers section
```

2. Test MCP binary directly:
```bash
tldr-mcp --version
```

3. Check logs (if available):
```bash
# Run in foreground for debugging
tldr-mcp 2>&1
```

### MCP tools not appearing in Claude Code

1. Verify MCP config:
```json
{
  "mcpServers": {
    "tldr": {
      "command": "tldr-mcp"
    }
  }
}
```

2. Restart Claude Code after config changes

3. Check [MCP integration guide](MCP.md) for detailed setup

## Performance Regression

### Analysis slower than before

1. Clear and rebuild cache:
```bash
tldr cache clear
tldr warm src/
```

2. Check for large files that may cause memory pressure:
```bash
tldr loc src/ --by-file | sort -k3 -n -r | head
```

3. Use daemon for frequently-analyzed codebases:
```bash
tldr daemon start
tldr warm src/
```

### High CPU usage

TLDR uses parallel processing where possible. Some commands are CPU-intensive:

- `calls` - builds call graph, parallel parsing
- `clones` - O(n²) similarity comparison
- `similar` - embedding computation

For large codebases, consider daemon mode to amortize cost:

```bash
tldr daemon start
tldr warm src/  # Run in background
```

## Getting Help

### Enable verbose output

```bash
# See detailed logs
tldr structure src/ -v

# For even more detail
TLDR_LOG=debug tldr structure src/
```

### Report a bug

1. Enable verbose mode
2. Capture the exact command and error
3. Create issue at [GitHub](https://github.com/parcadei/tldr-code/issues) with:
   - `tldr --version`
   - The failing command
   - Relevant output (`-v` flag)
   - Minimal reproduction case if possible

### Check known issues

[GitHub Issues](https://github.com/parcadei/tldr-code/issues) — Search before creating new issue.
