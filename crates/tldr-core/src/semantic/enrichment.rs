//! Embedding enrichment module
//!
//! Enriches code chunks with analysis context from all 5 layers
//! (AST, call graph, CFG, DFG, imports) to produce compact embedding text
//! suitable for 512-token models.
//!
//! # Architecture
//!
//! ```text
//! CodeChunk -> EmbeddingUnit -> build_embedding_text() -> String (~50 tokens)
//! ```
//!
//! The enriched text is transient -- used only for embedding generation,
//! not stored in cache or index.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

use crate::ast::imports::get_imports;
use crate::callgraph::builder_v2::build_project_call_graph_v2;
use crate::callgraph::cross_file_types::ProjectCallGraphV2;
use crate::callgraph::BuildConfig;
use crate::cfg::extractor::get_cfg_context;
use crate::dfg::extractor::get_dfg_context;
use crate::semantic::types::CodeChunk;
use crate::types::{BlockType, RefType};
use crate::Language;

/// Enriched representation of a code chunk for embedding.
///
/// Combines information from all 5 analysis layers into a compact
/// text representation (~50 tokens) suitable for 512-token models.
#[derive(Debug, Clone)]
pub struct EmbeddingUnit {
    /// Original chunk this was derived from
    pub chunk: CodeChunk,

    /// L1: Function/method signature (first line of the function)
    pub signature: String,

    /// L1: Docstring or leading comment
    pub docstring: String,

    /// L2: Top 5 functions this calls (callees)
    pub calls: Vec<String>,

    /// L2: Top 5 functions that call this (callers)
    pub called_by: Vec<String>,

    /// L3: Control flow summary (e.g., "complexity=4, branches=3, loops=1")
    pub cfg_summary: String,

    /// L4: Data flow summary (e.g., "vars=5, defs=3, uses=8")
    pub dfg_summary: String,

    /// L5: Import dependencies relevant to this function
    pub dependencies: String,
}

/// Pre-parsed file data to avoid redundant tree-sitter parsing.
///
/// Addresses PERF-1, PERF-2, PERF-5: parse each unique file exactly once,
/// then reuse the tree and source across all chunks from that file.
struct FileAnalysisCache {
    /// file_path -> full source content (for passing to CFG/DFG)
    file_sources: HashMap<PathBuf, String>,
    /// file_path -> formatted import dependencies string
    file_imports: HashMap<PathBuf, String>,
}

impl FileAnalysisCache {
    /// Build the cache by reading and parsing each unique file once.
    fn build(chunks: &[CodeChunk]) -> Self {
        let mut file_sources: HashMap<PathBuf, String> = HashMap::new();
        let mut file_imports: HashMap<PathBuf, String> = HashMap::new();

        // Collect unique file paths
        let unique_paths: Vec<PathBuf> = {
            let mut seen = std::collections::HashSet::new();
            chunks
                .iter()
                .filter(|c| seen.insert(c.file_path.clone()))
                .map(|c| c.file_path.clone())
                .collect()
        };

        for path in &unique_paths {
            // Read file content (for CFG/DFG analysis which needs full file)
            if let Ok(source) = std::fs::read_to_string(path) {
                file_sources.insert(path.clone(), source);
            }

            // Extract imports using the existing API (with catch_unwind for safety)
            let imports_str = std::panic::catch_unwind(AssertUnwindSafe(|| {
                // Detect language from the chunk (find first chunk with this path)
                let lang = chunks
                    .iter()
                    .find(|c| &c.file_path == path)
                    .map(|c| c.language);

                if let Some(lang) = lang {
                    match get_imports(path, lang) {
                        Ok(imports) => {
                            let modules: Vec<String> = imports
                                .iter()
                                .map(|imp| imp.module.clone())
                                .collect();
                            if modules.is_empty() {
                                String::new()
                            } else {
                                // Deduplicate and take top modules
                                let mut unique_modules: Vec<String> = modules;
                                unique_modules.sort();
                                unique_modules.dedup();
                                unique_modules.truncate(10);
                                unique_modules.join(", ")
                            }
                        }
                        Err(_) => String::new(),
                    }
                } else {
                    String::new()
                }
            }))
            .unwrap_or_default();

            if !imports_str.is_empty() {
                file_imports.insert(path.clone(), imports_str);
            }
        }

        FileAnalysisCache {
            file_sources,
            file_imports,
        }
    }
}

/// Build enriched text for embedding from all 5 analysis layers.
///
/// # Contract
/// - Output MUST be <= 2000 characters to fit in 512-token context
///   with room for model overhead.
/// - Each layer is optional: if analysis fails, that layer is silently omitted.
/// - Output format matches Python reference (semantic.py:212-262).
/// - Does NOT include raw source code.
///
/// # Output Format
/// ```text
/// Function: process_data
/// Signature: fn process_data(config: &Config) -> Result<Data>
/// Description: Processes raw data according to configuration
/// Calls: validate_input, transform, write_output
/// Called by: main, run_pipeline
/// Control flow: complexity=4, branches=3, loops=1
/// Data flow: vars=5, defs=3, uses=8
/// Dependencies: serde, tokio
/// ```
pub fn build_embedding_text(unit: &EmbeddingUnit) -> String {
    let mut parts = Vec::new();

    // Function name (from chunk or file path for file-level chunks)
    let name = unit
        .chunk
        .function_name
        .as_deref()
        .unwrap_or_else(|| {
            unit.chunk
                .file_path
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("unknown")
        });
    parts.push(format!("Function: {}", name));

    // L1: Signature (truncate to 200 chars if needed)
    if !unit.signature.is_empty() {
        let sig = if unit.signature.len() > 200 {
            &unit.signature[..200]
        } else {
            &unit.signature
        };
        parts.push(format!("Signature: {}", sig));
    }

    // L1: Description (docstring)
    if !unit.docstring.is_empty() {
        parts.push(format!("Description: {}", unit.docstring));
    }

    // L2: Calls (top 5 callees, only if non-empty)
    if !unit.calls.is_empty() {
        let top_calls: Vec<&str> = unit.calls.iter().take(5).map(|s| s.as_str()).collect();
        parts.push(format!("Calls: {}", top_calls.join(", ")));
    }

    // L2: Called by (top 5 callers, only if non-empty)
    if !unit.called_by.is_empty() {
        let top_callers: Vec<&str> = unit.called_by.iter().take(5).map(|s| s.as_str()).collect();
        parts.push(format!("Called by: {}", top_callers.join(", ")));
    }

    // L3: Control flow (only if non-empty)
    if !unit.cfg_summary.is_empty() {
        parts.push(format!("Control flow: {}", unit.cfg_summary));
    }

    // L4: Data flow (only if non-empty)
    if !unit.dfg_summary.is_empty() {
        parts.push(format!("Data flow: {}", unit.dfg_summary));
    }

    // L5: Dependencies (only if non-empty)
    if !unit.dependencies.is_empty() {
        parts.push(format!("Dependencies: {}", unit.dependencies));
    }

    let text = parts.join("\n");

    // Truncate to 2000 chars if needed (~512 tokens)
    if text.len() > 2000 {
        text[..2000].to_string()
    } else {
        text
    }
}

/// Build a CFG summary string from CfgInfo data.
///
/// Uses CfgInfo.cyclomatic_complexity directly (PERF-9: avoid separate
/// calculate_complexity call). Counts branches and loops from block types.
fn build_cfg_summary_from_source(
    source: &str,
    function_name: &str,
    language: Language,
) -> String {
    // Use full file source, not chunk content (C2 mitigation)
    match get_cfg_context(source, function_name, language) {
        Ok(cfg) => {
            let complexity = cfg.cyclomatic_complexity;
            let branches = cfg
                .blocks
                .iter()
                .filter(|b| b.block_type == BlockType::Branch)
                .count();
            let loops = cfg
                .blocks
                .iter()
                .filter(|b| {
                    b.block_type == BlockType::LoopHeader || b.block_type == BlockType::LoopBody
                })
                .count();
            format!(
                "complexity={}, branches={}, loops={}",
                complexity, branches, loops
            )
        }
        Err(_) => String::new(),
    }
}

/// Build a DFG summary string from DfgInfo data.
fn build_dfg_summary_from_source(
    source: &str,
    function_name: &str,
    language: Language,
) -> String {
    match get_dfg_context(source, function_name, language) {
        Ok(dfg) => {
            let vars = dfg.variables.len();
            let defs = dfg
                .refs
                .iter()
                .filter(|r| matches!(r.ref_type, RefType::Definition))
                .count();
            let uses = dfg
                .refs
                .iter()
                .filter(|r| matches!(r.ref_type, RefType::Use))
                .count();
            format!("vars={}, defs={}, uses={}", vars, defs, uses)
        }
        Err(_) => String::new(),
    }
}

/// Enrich a batch of CodeChunks into EmbeddingUnits.
///
/// # Contract
/// - Returns one EmbeddingUnit per input CodeChunk (1:1 mapping).
/// - If any analysis layer fails for a chunk, that layer is empty string/vec.
/// - Never panics. All analysis calls are wrapped in catch-unwind or Result.
/// - The call graph is built ONCE per language and reused for all chunks in the batch.
///
/// # Performance
/// - FileAnalysisCache: parse each unique file once (PERF-1, PERF-2, PERF-5)
/// - Call graph: built once per language, cached (PERF-3)
/// - CFG/DFG: use full file source, not chunk content (C2)
/// - Path normalization: strip root prefix for call graph lookups (C1)
/// - All analysis wrapped in catch_unwind (C6, C14)
pub fn enrich_chunks(chunks: &[CodeChunk], root: &Path) -> Vec<EmbeddingUnit> {
    // C8: Early return for empty input
    if chunks.is_empty() {
        return Vec::new();
    }

    // PERF-1/2/5: Build file analysis cache (parse each file once)
    let file_cache = FileAnalysisCache::build(chunks);

    // PERF-3: Group chunks by language, build one call graph per language
    let mut call_graphs: HashMap<Language, ProjectCallGraphV2> = HashMap::new();
    {
        let mut languages_seen = std::collections::HashSet::new();
        for chunk in chunks {
            if languages_seen.insert(chunk.language) {
                // C14: Wrap call graph build in catch_unwind
                let lang = chunk.language;
                let graph = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    let config = BuildConfig {
                        language: lang.as_str().to_string(),
                        ..Default::default()
                    };
                    // C7: Wrap in Result match -- on Err, proceed with empty call graph
                    match build_project_call_graph_v2(root, config) {
                        Ok(ir) => {
                            // Convert CallGraphIR edges to ProjectCallGraphV2
                            let mut graph = ProjectCallGraphV2::new();
                            for edge in ir.edges {
                                graph.add_edge(edge);
                            }
                            graph
                        }
                        Err(_) => ProjectCallGraphV2::new(),
                    }
                }))
                .unwrap_or_else(|_| ProjectCallGraphV2::new());

                call_graphs.insert(lang, graph);
            }
        }
    }

    // Enrich each chunk
    let result: Vec<EmbeddingUnit> = chunks
        .iter()
        .map(|chunk| {
            // C6: Wrap entire per-chunk enrichment in catch_unwind
            std::panic::catch_unwind(AssertUnwindSafe(|| {
                enrich_single_chunk(chunk, root, &file_cache, &call_graphs)
            }))
            .unwrap_or_else(|_| EmbeddingUnit {
                chunk: chunk.clone(),
                signature: String::new(),
                docstring: String::new(),
                calls: Vec::new(),
                called_by: Vec::new(),
                cfg_summary: String::new(),
                dfg_summary: String::new(),
                dependencies: String::new(),
            })
        })
        .collect();

    // Post-condition: 1:1 mapping invariant (C6)
    assert_eq!(
        result.len(),
        chunks.len(),
        "enrich_chunks must return exactly one EmbeddingUnit per input CodeChunk"
    );

    result
}

/// Enrich a single chunk with all 5 analysis layers.
fn enrich_single_chunk(
    chunk: &CodeChunk,
    root: &Path,
    file_cache: &FileAnalysisCache,
    call_graphs: &HashMap<Language, ProjectCallGraphV2>,
) -> EmbeddingUnit {
    // L1: Extract signature (first line of chunk content)
    let signature = chunk
        .content
        .lines()
        .next()
        .unwrap_or("")
        .to_string();

    // L1: Docstring -- for now use empty string (would need tree-sitter
    // comment node extraction; graceful degradation per spec)
    let docstring = String::new();

    // L2: Call graph lookups
    let (calls, called_by) = if let Some(func_name) = &chunk.function_name {
        if let Some(graph) = call_graphs.get(&chunk.language) {
            // C1: Normalize path relative to root before call graph lookups
            let rel_path = chunk
                .file_path
                .strip_prefix(root)
                .unwrap_or(&chunk.file_path);

            let callees: Vec<String> = graph
                .callees_of(rel_path, func_name)
                .map(|e| e.dst_func.clone())
                .take(5)
                .collect();

            let callers: Vec<String> = graph
                .callers_of(rel_path, func_name)
                .map(|e| e.src_func.clone())
                .take(5)
                .collect();

            (callees, callers)
        } else {
            (Vec::new(), Vec::new())
        }
    } else {
        // File-level chunk: skip L2
        (Vec::new(), Vec::new())
    };

    // L3: CFG summary
    // C2: Always pass full file content, not chunk.content
    let cfg_summary = if let Some(func_name) = &chunk.function_name {
        if let Some(source) = file_cache.file_sources.get(&chunk.file_path) {
            std::panic::catch_unwind(AssertUnwindSafe(|| {
                build_cfg_summary_from_source(source, func_name, chunk.language)
            }))
            .unwrap_or_default()
        } else {
            // Fallback: try using chunk content directly
            std::panic::catch_unwind(AssertUnwindSafe(|| {
                build_cfg_summary_from_source(&chunk.content, func_name, chunk.language)
            }))
            .unwrap_or_default()
        }
    } else {
        String::new()
    };

    // L4: DFG summary
    // C2: Always pass full file content, not chunk.content
    let dfg_summary = if let Some(func_name) = &chunk.function_name {
        if let Some(source) = file_cache.file_sources.get(&chunk.file_path) {
            std::panic::catch_unwind(AssertUnwindSafe(|| {
                build_dfg_summary_from_source(source, func_name, chunk.language)
            }))
            .unwrap_or_default()
        } else {
            // Fallback: try using chunk content directly
            std::panic::catch_unwind(AssertUnwindSafe(|| {
                build_dfg_summary_from_source(&chunk.content, func_name, chunk.language)
            }))
            .unwrap_or_default()
        }
    } else {
        String::new()
    };

    // L5: Dependencies (from file-level imports cache)
    let dependencies = file_cache
        .file_imports
        .get(&chunk.file_path)
        .cloned()
        .unwrap_or_default();

    EmbeddingUnit {
        chunk: chunk.clone(),
        signature,
        docstring,
        calls,
        called_by,
        cfg_summary,
        dfg_summary,
        dependencies,
    }
}

/// Compute a content hash based on source code only.
///
/// The hash is based on raw source code, NOT enriched text.
/// This is correct because cross-reference changes (new caller added)
/// do not change the function's identity.
pub fn content_hash_from_source(source: &str) -> String {
    format!("{:x}", md5::compute(source.as_bytes()))
}
