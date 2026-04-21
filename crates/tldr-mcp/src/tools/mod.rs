//! MCP Tool definitions and registry
//!
//! This module defines all 27 MCP tools for the TLDR code analysis system.
//! Tools are organized by category:
//!
//! - **Navigation**: tree, structure, extract, imports
//! - **Analysis**: calls, impact, dead, importers, arch
//! - **Flow**: cfg, dfg, slice, complexity
//! - **Search**: search, bm25, semantic
//! - **Context**: context, change_impact
//! - **Quality**: smells, maintainability, diagnostics, diff, debt
//! - **Security**: secrets, vuln, api_check
//! - **Composite**: health, todo, secure

pub mod ast;
pub mod callgraph;
pub mod flow;
pub mod quality;
pub mod search;
pub mod security;

use crate::cache::L1Cache;
use crate::protocol::{ToolDefinition, ToolsCallResult};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Tool handler function type
pub type ToolHandler = fn(Value) -> ToolsCallResult;

/// Default TTL for cached tool results (15 seconds).
///
/// Kept short because tools analyze filesystem state that can change between calls.
/// 15s prevents redundant re-computation during a single MCP conversation turn.
const DEFAULT_CACHE_TTL_SECS: u64 = 15;

/// Default maximum number of cached entries.
///
/// Capped at 200 to bound memory usage. Each entry stores a full `ToolsCallResult`
/// which can be large for project-wide analysis tools.
const DEFAULT_CACHE_MAX_ENTRIES: usize = 200;

/// Registry of all MCP tools
pub struct ToolRegistry {
    tools: HashMap<String, (ToolDefinition, ToolHandler)>,
    /// L1 in-process cache for tool results.
    /// Wrapped in RefCell because the server is single-threaded (blocking stdio loop).
    cache: RefCell<L1Cache>,
}

impl ToolRegistry {
    /// Create a new registry with all tools registered
    pub fn new() -> Self {
        let mut registry = Self {
            tools: HashMap::new(),
            cache: RefCell::new(L1Cache::new(
                Duration::from_secs(DEFAULT_CACHE_TTL_SECS),
                DEFAULT_CACHE_MAX_ENTRIES,
            )),
        };

        // Register all tools
        registry.register_ast_tools();
        registry.register_callgraph_tools();
        registry.register_flow_tools();
        registry.register_search_tools();
        registry.register_quality_tools();
        registry.register_security_tools();
        registry.register_composite_tools();

        registry
    }

    /// Register a tool
    fn register(&mut self, definition: ToolDefinition, handler: ToolHandler) {
        self.tools
            .insert(definition.name.clone(), (definition, handler));
    }

    /// Get all tool definitions
    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        let mut tools: Vec<_> = self.tools.values().map(|(def, _)| def.clone()).collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        tools
    }

    /// Get tool count
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Call a tool by name, with L1 cache for deterministic, non-project-wide tools.
    ///
    /// Cache behavior:
    /// - Project-wide tools (`tldr_calls`, `tldr_dead`, `tldr_health`, `tldr_todo`,
    ///   `tldr_secure`, `tldr_arch`) are excluded from caching because they produce
    ///   large results and are rarely called twice with the same args.
    /// - Error results are never cached to allow retries.
    /// - Cache is keyed on `tool_name:sorted_args_json` for deterministic lookup.
    pub fn call_tool(&self, name: &str, arguments: Value) -> ToolsCallResult {
        // Project-wide tools excluded from L1 cache (large results, rarely repeated)
        let skip_cache = matches!(
            name,
            "tldr_calls" | "tldr_dead" | "tldr_health" | "tldr_todo" | "tldr_secure" | "tldr_arch"
        );

        // Check L1 cache for a fresh hit
        let cache_key = if skip_cache {
            String::new()
        } else {
            let key = L1Cache::cache_key(name, &arguments);
            if let Some(cached) = self.cache.borrow().get(&key) {
                return cached.clone();
            }
            key
        };

        // Dispatch to tool handler
        let result = match self.tools.get(name) {
            Some((_, handler)) => {
                // M15: Catch panics at MCP boundary to prevent server crashes
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(arguments)))
                    .unwrap_or_else(|_| {
                        ToolsCallResult::error("Internal error: tool handler panicked")
                    })
            }
            None => ToolsCallResult::error(format!("Unknown tool: {}", name)),
        };

        // Cache successful results only (errors should be retryable)
        if !skip_cache && result.is_error != Some(true) {
            self.cache
                .borrow_mut()
                .insert(cache_key, result.clone());
        }

        result
    }

    // =========================================================================
    // Cache Management
    // =========================================================================

    /// Invalidate a specific cache entry by tool name and arguments.
    ///
    /// Used when external state changes are detected (e.g., file modification
    /// notifications from the daemon).
    pub fn cache_invalidate(&self, name: &str, arguments: &Value) {
        let key = L1Cache::cache_key(name, arguments);
        self.cache.borrow_mut().invalidate(&key);
    }

    /// Clear the entire L1 cache.
    ///
    /// Useful when a project-wide change is detected (e.g., git checkout)
    /// that would invalidate all cached results.
    pub fn cache_clear(&self) {
        self.cache.borrow_mut().clear();
    }

    /// Return the number of entries currently in the L1 cache.
    pub fn cache_len(&self) -> usize {
        self.cache.borrow().len()
    }

    /// Return whether the L1 cache is empty.
    pub fn cache_is_empty(&self) -> bool {
        self.cache.borrow().is_empty()
    }

    // =========================================================================
    // AST Tools (Navigation) - tree, structure, extract, imports
    // =========================================================================
    fn register_ast_tools(&mut self) {
        // tldr_tree
        self.register(
            ToolDefinition {
                name: "tldr_tree".to_string(),
                description: "Get file tree structure for a directory. Returns hierarchical view of files and folders.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Root directory to scan"
                        },
                        "extensions": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "File extensions to include (e.g., [\".py\", \".ts\"])"
                        },
                        "exclude_hidden": {
                            "type": "boolean",
                            "description": "Whether to exclude hidden files (default: true)"
                        }
                    },
                    "required": ["path"]
                }),
            },
            ast::handle_tree,
        );

        // tldr_structure
        self.register(
            ToolDefinition {
                name: "tldr_structure".to_string(),
                description: "Extract code structure (functions, classes, imports) from files. Provides a codemap overview.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory or file to analyze"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language (python, typescript, go, rust, java)"
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum number of files to return (0 = unlimited)"
                        }
                    },
                    "required": ["path", "language"]
                }),
            },
            ast::handle_structure,
        );

        // tldr_extract
        self.register(
            ToolDefinition {
                name: "tldr_extract".to_string(),
                description: "Extract complete module information from a single file including functions, classes, docstrings, and intra-file call graph.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "File path to extract from"
                        },
                        "base_path": {
                            "type": "string",
                            "description": "Base path for relative imports"
                        }
                    },
                    "required": ["file"]
                }),
            },
            ast::handle_extract,
        );

        // tldr_imports
        self.register(
            ToolDefinition {
                name: "tldr_imports".to_string(),
                description: "Parse import statements from a source file.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "File path to parse imports from"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["file"]
                }),
            },
            ast::handle_imports,
        );
    }

    // =========================================================================
    // Call Graph Tools (Analysis) - calls, impact, dead, importers, arch
    // =========================================================================
    fn register_callgraph_tools(&mut self) {
        // tldr_calls
        self.register(
            ToolDefinition {
                name: "tldr_calls".to_string(),
                description:
                    "Build cross-file call graph for a project. Shows which functions call which."
                        .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["path", "language"]
                }),
            },
            callgraph::handle_calls,
        );

        // tldr_impact
        self.register(
            ToolDefinition {
                name: "tldr_impact".to_string(),
                description: "Find all callers of a function (reverse call graph traversal). Useful for understanding the impact of changes.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "function": {
                            "type": "string",
                            "description": "Function name to analyze"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        },
                        "depth": {
                            "type": "integer",
                            "description": "Maximum traversal depth (default: 3)"
                        },
                        "file": {
                            "type": "string",
                            "description": "Filter to specific file"
                        }
                    },
                    "required": ["path", "function", "language"]
                }),
            },
            callgraph::handle_impact,
        );

        // tldr_dead
        self.register(
            ToolDefinition {
                name: "tldr_dead".to_string(),
                description: "Find dead code (functions that are never called). Helps identify code that can be safely removed.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        },
                        "entry_points": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Custom entry point patterns to exclude"
                        }
                    },
                    "required": ["path", "language"]
                }),
            },
            callgraph::handle_dead,
        );

        // tldr_importers
        self.register(
            ToolDefinition {
                name: "tldr_importers".to_string(),
                description: "Find all files that import a given module.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "module": {
                            "type": "string",
                            "description": "Module name to search for"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["path", "module", "language"]
                }),
            },
            callgraph::handle_importers,
        );

        // tldr_arch
        self.register(
            ToolDefinition {
                name: "tldr_arch".to_string(),
                description: "Analyze codebase architecture to detect layers (entry, service, utility) and circular dependencies.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["path", "language"]
                }),
            },
            callgraph::handle_arch,
        );
    }

    // =========================================================================
    // Flow Tools - cfg, dfg, slice, complexity
    // =========================================================================
    fn register_flow_tools(&mut self) {
        // tldr_cfg
        self.register(
            ToolDefinition {
                name: "tldr_cfg".to_string(),
                description: "Extract control flow graph for a function. Shows basic blocks and control flow edges.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "Source file path"
                        },
                        "function": {
                            "type": "string",
                            "description": "Function name"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["file", "function"]
                }),
            },
            flow::handle_cfg,
        );

        // tldr_complexity
        self.register(
            ToolDefinition {
                name: "tldr_complexity".to_string(),
                description:
                    "Calculate cyclomatic and cognitive complexity metrics for a function."
                        .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "Source file path"
                        },
                        "function": {
                            "type": "string",
                            "description": "Function name"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["file", "function"]
                }),
            },
            flow::handle_complexity,
        );

        // tldr_dfg
        self.register(
            ToolDefinition {
                name: "tldr_dfg".to_string(),
                description: "Extract data flow graph for a function. Shows variable definitions, uses, and def-use chains.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "Source file path"
                        },
                        "function": {
                            "type": "string",
                            "description": "Function name"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["file", "function"]
                }),
            },
            flow::handle_dfg,
        );

        // tldr_slice
        self.register(
            ToolDefinition {
                name: "tldr_slice".to_string(),
                description: "Compute program slice from a line. Shows what affects or is affected by that line.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "Source file path"
                        },
                        "function": {
                            "type": "string",
                            "description": "Function name"
                        },
                        "line": {
                            "type": "integer",
                            "description": "Line number to slice from"
                        },
                        "direction": {
                            "type": "string",
                            "enum": ["backward", "forward"],
                            "description": "Slice direction (backward = what affects this line, forward = what this line affects)"
                        },
                        "variable": {
                            "type": "string",
                            "description": "Optional variable to trace"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["file", "function", "line"]
                }),
            },
            flow::handle_slice,
        );

        // tldr_pdg
        self.register(
            ToolDefinition {
                name: "tldr_pdg".to_string(),
                description: "Extract program dependence graph combining CFG and DFG.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "Source file path"
                        },
                        "function": {
                            "type": "string",
                            "description": "Function name"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["file", "function"]
                }),
            },
            flow::handle_pdg,
        );
    }

    // =========================================================================
    // Search Tools - search, bm25, semantic
    // =========================================================================
    fn register_search_tools(&mut self) {
        // tldr_search
        self.register(
            ToolDefinition {
                name: "tldr_search".to_string(),
                description: "Search files for a regex pattern.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Regex pattern to search for"
                        },
                        "path": {
                            "type": "string",
                            "description": "Directory or file to search in"
                        },
                        "extensions": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "File extensions to include"
                        },
                        "context_lines": {
                            "type": "integer",
                            "description": "Number of context lines (default: 0)"
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum results (default: 100)"
                        }
                    },
                    "required": ["pattern", "path"]
                }),
            },
            search::handle_search,
        );

        // tldr_bm25
        self.register(
            ToolDefinition {
                name: "tldr_bm25".to_string(),
                description: "BM25 keyword search over code. Ranks results by relevance."
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query"
                        },
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        },
                        "top_k": {
                            "type": "integer",
                            "description": "Number of results (default: 10)"
                        }
                    },
                    "required": ["query", "path"]
                }),
            },
            search::handle_bm25,
        );

        // tldr_semantic
        self.register(
            ToolDefinition {
                name: "tldr_semantic".to_string(),
                description: "Hybrid search combining BM25 and semantic embeddings using RRF."
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query"
                        },
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        },
                        "top_k": {
                            "type": "integer",
                            "description": "Number of results (default: 10)"
                        }
                    },
                    "required": ["query", "path"]
                }),
            },
            search::handle_semantic,
        );
    }

    // =========================================================================
    // Quality Tools - smells, maintainability, diagnostics, change_impact, diff, debt
    // =========================================================================
    fn register_quality_tools(&mut self) {
        // tldr_context
        self.register(
            ToolDefinition {
                name: "tldr_context".to_string(),
                description: "Get token-efficient LLM context from an entry point. Achieves ~95% token savings compared to reading full files.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "entry_point": {
                            "type": "string",
                            "description": "Function to start from"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        },
                        "depth": {
                            "type": "integer",
                            "description": "Traversal depth (default: 2)"
                        },
                        "include_docstrings": {
                            "type": "boolean",
                            "description": "Include docstrings in output"
                        }
                    },
                    "required": ["path", "entry_point", "language"]
                }),
            },
            quality::handle_context,
        );

        // tldr_change_impact
        self.register(
            ToolDefinition {
                name: "tldr_change_impact".to_string(),
                description: "Find tests affected by changed files. Enables selective test running.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        },
                        "changed_files": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "List of changed file paths (auto-detected from git if not provided)"
                        }
                    },
                    "required": ["path", "language"]
                }),
            },
            quality::handle_change_impact,
        );

        // tldr_smells
        self.register(
            ToolDefinition {
                name: "tldr_smells".to_string(),
                description: "Detect code smells (God Class, Long Method, Long Parameter List, etc.)".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File or directory to analyze"
                        },
                        "threshold": {
                            "type": "string",
                            "enum": ["strict", "default", "relaxed"],
                            "description": "Threshold preset (default: default)"
                        },
                        "smell_type": {
                            "type": "string",
                            "enum": ["GodClass", "LongMethod", "LongParameterList", "FeatureEnvy", "DataClumps"],
                            "description": "Filter to specific smell type"
                        },
                        "suggest": {
                            "type": "boolean",
                            "description": "Include suggestions for fixing smells"
                        }
                    },
                    "required": ["path"]
                }),
            },
            quality::handle_smells,
        );

        // tldr_maintainability
        self.register(
            ToolDefinition {
                name: "tldr_maintainability".to_string(),
                description: "Calculate Maintainability Index (MI) score for files.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File or directory to analyze"
                        },
                        "include_halstead": {
                            "type": "boolean",
                            "description": "Include Halstead metrics"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["path"]
                }),
            },
            quality::handle_maintainability,
        );

        // tldr_diagnostics
        self.register(
            ToolDefinition {
                name: "tldr_diagnostics".to_string(),
                description: "Run type checking and linting (pyright/ruff for Python).".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File or directory to check"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["path"]
                }),
            },
            quality::handle_diagnostics,
        );

        // tldr_diff
        self.register(
            ToolDefinition {
                name: "tldr_diff".to_string(),
                description: "Semantic diff between two versions of code.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "old": {
                            "type": "string",
                            "description": "Path to old version"
                        },
                        "new": {
                            "type": "string",
                            "description": "Path to new version"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["old", "new"]
                }),
            },
            quality::handle_diff,
        );

        // tldr_debt
        self.register(
            ToolDefinition {
                name: "tldr_debt".to_string(),
                description: "Estimate technical debt based on complexity and smells.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["path"]
                }),
            },
            quality::handle_debt,
        );
    }

    // =========================================================================
    // Security Tools - secrets, vuln, api_check
    // =========================================================================
    fn register_security_tools(&mut self) {
        // tldr_secrets
        self.register(
            ToolDefinition {
                name: "tldr_secrets".to_string(),
                description: "Scan for hardcoded secrets (API keys, passwords, private keys).".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File or directory to scan"
                        },
                        "entropy_threshold": {
                            "type": "number",
                            "description": "Entropy threshold for high-entropy string detection (default: 4.5)"
                        },
                        "include_test": {
                            "type": "boolean",
                            "description": "Include test files in scan"
                        },
                        "severity_filter": {
                            "type": "string",
                            "enum": ["low", "medium", "high", "critical"],
                            "description": "Minimum severity to report"
                        }
                    },
                    "required": ["path"]
                }),
            },
            security::handle_secrets,
        );

        // tldr_vuln
        self.register(
            ToolDefinition {
                name: "tldr_vuln".to_string(),
                description: "Detect vulnerabilities via taint analysis (SQL injection, XSS, command injection, etc.)".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File or directory to scan"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        },
                        "vuln_type": {
                            "type": "string",
                            "enum": ["SqlInjection", "Xss", "CommandInjection", "PathTraversal", "Ssrf", "Deserialization"],
                            "description": "Filter to specific vulnerability type"
                        }
                    },
                    "required": ["path"]
                }),
            },
            security::handle_vuln,
        );

        // tldr_api_check
        self.register(
            ToolDefinition {
                name: "tldr_api_check".to_string(),
                description: "Check for insecure API usage patterns.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File or directory to check"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["path"]
                }),
            },
            security::handle_api_check,
        );
    }

    // =========================================================================
    // Composite Tools - health, todo, secure
    // =========================================================================
    fn register_composite_tools(&mut self) {
        // tldr_health
        self.register(
            ToolDefinition {
                name: "tldr_health".to_string(),
                description: "Health dashboard combining multiple metrics (complexity, smells, maintainability).".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["path"]
                }),
            },
            quality::handle_health,
        );

        // tldr_todo
        self.register(
            ToolDefinition {
                name: "tldr_todo".to_string(),
                description: "Generate action items from code analysis (high complexity functions, dead code, security issues).".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["path"]
                }),
            },
            quality::handle_todo,
        );

        // tldr_secure
        self.register(
            ToolDefinition {
                name: "tldr_secure".to_string(),
                description: "Security summary combining secrets scan and vulnerability detection."
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Project root directory"
                        },
                        "language": {
                            "type": "string",
                            "description": "Programming language"
                        }
                    },
                    "required": ["path"]
                }),
            },
            security::handle_secure,
        );
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to extract a required string argument
pub fn get_required_string(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Missing required argument: {}", key))
}

/// Helper to extract an optional string argument
pub fn get_optional_string(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Helper to extract an optional integer argument
pub fn get_optional_int(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64())
}

/// Helper to extract an optional boolean argument
pub fn get_optional_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

/// Helper to extract an optional string array argument
pub fn get_optional_string_array(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key).and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    })
}

/// Helper to convert path string to PathBuf
pub fn to_path(path: &str) -> PathBuf {
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_has_30_tools() {
        // 27 tools from spec + 3 additional composite tools (api_check, debt, diagnostics)
        let registry = ToolRegistry::new();
        assert_eq!(registry.tool_count(), 30);
    }

    #[test]
    fn test_unknown_tool_returns_error() {
        let registry = ToolRegistry::new();
        let result = registry.call_tool("nonexistent_tool", json!({}));
        assert!(result.is_error == Some(true));
    }

    #[test]
    fn test_list_tools_returns_sorted() {
        let registry = ToolRegistry::new();
        let tools = registry.list_tools();
        let names: Vec<_> = tools.iter().map(|t| &t.name).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    // -----------------------------------------------------------------------
    // Benchmark: full call_tool() cache hit path (target: <15us)
    // -----------------------------------------------------------------------
    #[test]
    fn bench_call_tool_cache_hit() {
        use std::time::{Duration, Instant};

        let registry = ToolRegistry::new();

        // Use CARGO_MANIFEST_DIR as the path — it exists and is small enough
        // for tldr_tree to return a successful (non-error) result that gets cached.
        let args = json!({"path": env!("CARGO_MANIFEST_DIR")});
        let tool_name = "tldr_tree";

        // First call: cache miss — computes the real result and caches it
        let first_result = registry.call_tool(tool_name, args.clone());
        assert!(
            first_result.is_error != Some(true),
            "First call must succeed to populate cache. Got error: {:?}",
            first_result.content.first().map(|c| &c.text)
        );

        // Verify cache was populated
        assert!(
            !registry.cache_is_empty(),
            "Cache should have at least one entry after first call"
        );

        // Warm-up: one more cached call to prime any branch predictor / instruction cache
        let _ = registry.call_tool(tool_name, args.clone());

        // Measure: 10,000 cache hit calls through the full call_tool() path
        let iterations = 10_000u32;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = std::hint::black_box(registry.call_tool(tool_name, args.clone()));
        }
        let elapsed = start.elapsed();
        let per_call = elapsed / iterations;

        eprintln!(
            "call_tool cache hit: {:?} per call ({} iterations in {:?})",
            per_call, iterations, elapsed
        );

        // The full path includes:
        //   1. matches!() check for skip_cache (~0ns, compile-time)
        //   2. L1Cache::cache_key() — JSON key sorting + format!()
        //   3. RefCell::borrow() + HashMap::get() + TTL check
        //   4. ToolsCallResult::clone()
        // Target: <15us total (matching mock benchmark projections)
        assert!(
            per_call < Duration::from_micros(15),
            "call_tool cache hit too slow: {:?} (target: <15us)",
            per_call
        );
    }

    // -----------------------------------------------------------------------
    // Benchmark: call_tool() cache hit — breakdown of clone cost
    // -----------------------------------------------------------------------
    #[test]
    fn bench_call_tool_cache_hit_clone_cost() {
        use std::time::{Duration, Instant};

        let registry = ToolRegistry::new();

        // Use a tool that produces a larger result to stress-test clone overhead
        let args = json!({"path": env!("CARGO_MANIFEST_DIR"), "language": "rust"});
        let tool_name = "tldr_structure";

        // First call: populate cache
        let first_result = registry.call_tool(tool_name, args.clone());
        // This may error if no .rs files found in the manifest dir itself;
        // in that case the result won't be cached (is_error check).
        // Fall back to tldr_tree if structure errors.
        let (tool_name, args) = if first_result.is_error == Some(true) {
            ("tldr_tree", json!({"path": env!("CARGO_MANIFEST_DIR")}))
        } else {
            (tool_name, args)
        };

        // Re-populate if we changed tools
        if first_result.is_error == Some(true) {
            let _ = registry.call_tool(tool_name, args.clone());
        }

        // Measure the size of the cached result for context
        let result = registry.call_tool(tool_name, args.clone());
        let result_size: usize = result.content.iter().map(|c| c.text.len()).sum();
        eprintln!("Cached result size: {} bytes", result_size);

        // Measure
        let iterations = 10_000u32;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = std::hint::black_box(registry.call_tool(tool_name, args.clone()));
        }
        let elapsed = start.elapsed();
        let per_call = elapsed / iterations;

        eprintln!(
            "call_tool cache hit (result_size={}B): {:?} per call ({} iterations in {:?})",
            result_size, per_call, iterations, elapsed
        );

        // Even with larger results, should stay under 15us.
        // If clone cost dominates, this test will reveal it.
        assert!(
            per_call < Duration::from_micros(15),
            "call_tool cache hit too slow with {}B result: {:?} (target: <15us)",
            result_size,
            per_call
        );
    }
}
