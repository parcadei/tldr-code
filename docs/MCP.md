# MCP Server Integration

TLDR includes a Model Context Protocol (MCP) server for integration with Claude Code and other MCP-compatible clients.

## What is MCP?

The [Model Context Protocol](https://modelcontextprotocol.io/) is a standard interface for connecting AI assistants to external tools and data sources. TLDR's MCP server exposes code analysis capabilities to any MCP client.

## Architecture

```
┌─────────────────┐     JSON-RPC 2.0      ┌─────────────────┐
│   Claude Code   │ ◄──────────────────► │    tldr-mcp     │
│   (or other    │     stdio transport   │   (MCP server)  │
│   MCP client)   │                       │                 │
└─────────────────┘                       └────────┬────────┘
                                                    │
                                                    ▼
                                           ┌─────────────────┐
                                           │   tldr-core     │
                                           │  (analysis engine)│
                                           └─────────────────┘
```

## Installation

### 1. Build the MCP Server

```bash
cargo build --release -p tldr-mcp
```

The binary will be at: `target/release/tldr-mcp`

### 2. Configure Your MCP Client

#### Claude Code

Add to your Claude Code MCP configuration:

```json
{
  "mcpServers": {
    "tldr": {
      "command": "/path/to/tldr-mcp",
      "args": ["--project", "/path/to/your/codebase"]
    }
  }
}
```

Or using environment variable in config file:

```json
{
  "mcpServers": {
    "tldr": {
      "command": "tldr-mcp",
      "env": {
        "TLDR_PROJECT_ROOT": "/path/to/your/codebase"
      }
    }
  }
}
```

#### Other MCP Clients

The server uses stdio transport and JSON-RPC 2.0 protocol, making it compatible with any MCP client.

## Available Tools

The MCP server exposes these tool categories:

### AST Analysis (L1)

| Tool | Description | Arguments |
|------|-------------|-----------|
| `tldr_tree` | Show file tree structure | `path?`, `extensions?`, `include_hidden?` |
| `tldr_structure` | Extract code structure | `path?`, `max_results?` |
| `tldr_extract` | Complete module info | `file` |
| `tldr_imports` | Parse imports | `file` |

### Call Graph (L2)

| Tool | Description | Arguments |
|------|-------------|-----------|
| `tldr_calls` | Build call graph | `path?`, `max_items?` |
| `tldr_impact` | Find callers of function | `function`, `path?`, `depth?` |
| `tldr_dead` | Find dead code | `path?` |
| `tldr_refs` | Find symbol references | `symbol`, `path?` |

### Data Flow (L3-L4)

| Tool | Description | Arguments |
|------|-------------|-----------|
| `tldr_reaching_defs` | Reaching definitions | `file`, `function` |
| `tldr_available` | Available expressions | `file`, `function` |
| `tldr_dead_stores` | Dead store detection | `file`, `function` |
| `tldr_slice` | Program slice | `file`, `function`, `line` |

### Search

| Tool | Description | Arguments |
|------|-------------|-----------|
| `tldr_search` | BM25 search | `query`, `path?` |
| `tldr_semantic` | Natural language search | `query`, `path?` |
| `tldr_context` | LLM context from entry | `entry`, `project?` |

### Quality

| Tool | Description | Arguments |
|------|-------------|-----------|
| `tldr_smells` | Code smell detection | `path?` |
| `tldr_complexity` | Cyclomatic complexity | `file`, `function` |
| `tldr_health` | Health dashboard | `path?` |
| `tldr_hotspots` | Churn x complexity | `path?` |

### Security

| Tool | Description | Arguments |
|------|-------------|-----------|
| `tldr_taint` | Taint flow analysis | `file`, `function` |
| `tldr_vuln` | Vulnerability scan | `path?` |
| `tldr_api_check` | API misuse patterns | `path?` |
| `tldr_secure` | Security dashboard | `path?` |

## Tool Definitions

Tool definitions are in [`crates/tldr-mcp/src/tools/`](https://github.com/parcadei/tldr-code/tree/main/crates/tldr-mcp/src/tools):

| File | Category |
|------|----------|
| `ast.rs` | L1 AST analysis |
| `callgraph.rs` | L2 call graph analysis |
| `flow.rs` | L3-L4 data flow analysis |
| `search.rs` | Search commands |
| `quality.rs` | Quality metrics |
| `security.rs` | Security analysis |

## Usage Examples

### Claude Code

Once configured, use natural language:

```
What's the call graph for the auth module?
What functions call parse_config?
Find dead code in the utils directory.
Analyze taint flows in the user input handler.
```

### Direct JSON-RPC

The MCP server accepts standard JSON-RPC 2.0 requests:

```bash
# Initialize
echo '{"jsonrpc":"2.0","id":1,"method":"initialize"}' | tldr-mcp

# List tools
echo '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | tldr-mcp

# Call a tool
echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tldr_structure","arguments":{"path":"src"}}}' | tldr-mcp
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `TLDR_PROJECT_ROOT` | Default project root | Current directory |
| `TLDR_LOG` | Log level (debug, info, warn, error) | `info` |
| `TLDR_CACHE_DIR` | Cache directory | Platform-specific |

### Tool Filtering

You can limit which tools are exposed via MCP config:

```json
{
  "mcpServers": {
    "tldr": {
      "command": "tldr-mcp",
      "env": {
        "TLDR_PROJECT_ROOT": "/path/to/project",
        "TLDR_TOOLS": "ast,callgraph,security"
      }
    }
  }
}
```

## Caching

The MCP server uses a two-level cache:

1. **L1 In-process cache** (memory) — Tool results cached for session
2. **L2 Persistent cache** (disk) — Shared across sessions via daemon

Cache key: `hash(tool_name + arguments + file_mtimes)`

Invalidation: Automatic on file modification (via `tldr daemon notify`)

## Error Handling

Errors are returned as JSON-RPC error responses:

```json
{
  "jsonrpc": "2.0",
  "error": {
    "code": -32603,
    "message": "Failed to parse file",
    "data": {
      "file": "src/main.py",
      "error": "Unsupported syntax in Python 3.12"
    }
  }
}
```

Error codes:
- `-32603` — Internal error (parse failed, etc.)
- `-32602` — Invalid arguments
- `-32600` — Invalid request

## Development

### Adding a New Tool

1. Add tool definition in `tools/*.rs`:

```rust
#[derive(Tool)]
pub struct MyTool {
    pub name: String,
    pub description: String,
    pub arguments: Vec<Argument>,
}

impl Tool for MyTool {
    fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<serde_json::Value> {
        // Call tldr-core function
        // Return JSON result
    }
}
```

2. Register in `tools/mod.rs`

3. Rebuild: `cargo build -p tldr-mcp`

### Testing MCP Server

```bash
# Run tests
cargo test -p tldr-mcp

# Manual test with echo
echo '{"jsonrpc":"2.0","id":0,"method":"initialize"}' | target/debug/tldr-mcp
```

## Troubleshooting

### Server won't start

1. Check binary exists and is executable:
```bash
ls -la target/release/tldr-mcp
./target/release/tldr-mcp --version
```

2. Check logs:
```bash
TLDR_LOG=debug ./target/release/tldr-mcp 2>&1
```

### Tools not appearing

1. Verify JSON-RPC connection:
```bash
echo '{"jsonrpc":"2.0","id":0,"method":"initialize"}' | tldr-mcp
```

2. Check tool registry initialization logs

### Slow tool execution

1. Pre-warm the cache:
```bash
tldr daemon start
tldr warm /path/to/project
```

2. MCP client should then hit L1 cache

## See Also

- [TLDR Architecture](ARCHITECTURE.md) — How the analysis engine works
- [Command Reference](commands/) — Detailed command documentation
- [MCP Protocol Spec](https://modelcontextprotocol.io/spec) — Protocol specification
