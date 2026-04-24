# Installation

## Requirements

- **Rust** 1.70+ (for building from source)
- **Git** (for version control integration features)
- **Tree-sitter grammars** (included, pinned versions)

## Install from Binary (Recommended)

Download the latest binary for macOS or Linux from [GitHub Releases](https://github.com/parcadei/tldr-code/releases). Windows users should install from source (see below).

```bash
# macOS (Apple Silicon)
curl -L https://github.com/parcadei/tldr-code/releases/latest/download/tldr-aarch64-apple-darwin.gz | gunzip > tldr
chmod +x tldr
sudo mv tldr /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/parcadei/tldr-code/releases/latest/download/tldr-x86_64-apple-darwin.gz | gunzip > tldr
chmod +x tldr
sudo mv tldr /usr/local/bin/

# Linux (x86_64)
curl -L https://github.com/parcadei/tldr-code/releases/latest/download/tldr-x86_64-unknown-linux-musl.gz | gunzip > tldr
chmod +x tldr
sudo mv tldr /usr/local/bin/

# Linux (ARM64)
curl -L https://github.com/parcadei/tldr-code/releases/latest/download/tldr-aarch64-unknown-linux-musl.gz | gunzip > tldr
chmod +x tldr
sudo mv tldr /usr/local/bin/
```

Verify the installation:

```bash
tldr --version
```

## Install from Source

### 1. Clone the repository

```bash
git clone https://github.com/parcadei/tldr-code.git
cd tldr-code
```

### 2. Build the project

```bash
# Debug build (faster, larger)
cargo build

# Release build (slower, optimized)
cargo build --release
```

The binary will be at:
- Debug: `target/debug/tldr`
- Release: `target/release/tldr`

### 3. Install to PATH

```bash
# For release build
cp target/release/tldr ~/.local/bin/

# Or add to PATH temporarily
export PATH="$PWD/target/release:$PATH"
```

## Dependencies

### Tree-sitter Grammars (Auto-included)

TLDR uses pinned tree-sitter versions to ensure consistent AST parsing:

| Language | Package | Version |
|----------|---------|---------|
| Python | tree-sitter-python | 0.23.6 |
| TypeScript | tree-sitter-typescript | 0.23.2 |
| JavaScript | tree-sitter-typescript | 0.23.2 |
| Go | tree-sitter-go | 0.23.4 |
| Rust | tree-sitter-rust | 0.23.3 |
| Java | tree-sitter-java | 0.23.5 |
| C | tree-sitter-c | 0.23.4 |
| C++ | tree-sitter-cpp | 0.23.4 |
| Ruby | tree-sitter-ruby | 0.23.1 |
| Kotlin | tree-sitter-kotlin-ng | 1.1.0 |
| Swift | tree-sitter-swift | 0.7.1 |
| C# | tree-sitter-c-sharp | 0.23.1 |
| Scala | tree-sitter-scala | 0.24.0 |
| PHP | tree-sitter-php | 0.23.11 |
| Lua | tree-sitter-lua | 0.2.0 |
| Luau | tree-sitter-luau | 1.2.0 |
| Elixir | tree-sitter-elixir | 0.3.4 |
| OCaml | tree-sitter-ocaml | 0.24.2 |

**Important**: Do not use `^` or `~` version specifiers for grammars — exact versions (`=X.Y.Z`) are required to prevent AST node type mismatches.

See [GRAMMAR_COMPATIBILITY.md](https://github.com/parcadei/tldr-code/blob/main/docs/GRAMMAR_COMPATIBILITY.md) for version rationale.

### Optional: Diagnostic Tools

Some commands (`doctor`, `diagnostics`) require external tools:

| Language | Tools |
|----------|-------|
| Python | `pyright`, `ruff`, `mypy` |
| TypeScript | `typescript-language-server`, `tsc` |
| Go | `gopls`, `golangci-lint` |
| Rust | `rustc`, `cargo` |
| Java | `checkstyle`, `spotbugs` |

Install with `tldr doctor --install <language>`:

```bash
tldr doctor --install python  # Install Python tools
tldr doctor --install rust  # Install Rust tools
```

## Verifying Installation

```bash
# Check version
tldr --version

# Run self-diagnostics
tldr doctor

# Test basic functionality
tldr structure . --format text
```

## Platform Notes

### macOS

- TLDR is notarized and signed where possible
- First run may require approval in System Preferences > Security & Privacy

### Linux

- Static binaries (musl target) work on most distros without glibc compatibility issues
- For `daemon` commands, you may need to increase shared memory limits for large codebases

### Windows

- Windows support is available via TCP sockets (Unix sockets not supported)
- Daemon uses `127.0.0.1` with auto-assigned port

## Updating

```bash
# Update from binary release
# Download the new binary and replace the old one

# Update from source
git pull
cargo build --release
```

## Uninstalling

```bash
# Remove the binary
rm ~/.local/bin/tldr
# or
sudo rm /usr/local/bin/tldr
```

To also remove cached data:

```bash
# Remove cache directory
rm -rf ~/.cache/tldr
rm -rf ~/.local/share/tldr
```
