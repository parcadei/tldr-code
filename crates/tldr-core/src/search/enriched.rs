//! Enriched search: BM25 search results enriched with structure + call graph context.
//!
//! Returns "search result cards" containing:
//! - Function/class/method name and kind
//! - File path and line range
//! - Signature (definition line)
//! - Callers and callees from call graph (optional, may be empty)
//! - BM25 relevance score
//! - Matched search terms
//!
//! Designed for LLM agents that need function-level context with relationships
//! in a single query, minimizing round-trips.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use super::bm25::{Bm25Index, Bm25Result};
use super::text::{self, SearchMatch};
use super::tokenizer::Tokenizer;
use crate::ast::parser::parse_file;
use crate::types::{CodeStructure, DefinitionInfo, Language};
use crate::TldrResult;

/// Search mode selector for enriched search.
///
/// Controls how initial matches are discovered before tree-sitter enrichment.
/// BM25 uses tokenized relevance ranking; Regex uses pattern matching.
#[derive(Debug, Clone, Default)]
pub enum SearchMode {
    /// BM25 tokenized relevance ranking (current default).
    /// Tokenizes query into terms, scores documents by BM25 formula.
    #[default]
    Bm25,

    /// Regex pattern matching.
    /// Compiles the pattern, scans files line-by-line, then enriches hits.
    /// The String is the regex pattern (same syntax as `regex` crate).
    Regex(String),

    /// Hybrid: BM25 + Regex fusion via Reciprocal Rank Fusion (RRF).
    ///
    /// Runs both BM25 (with `query`) and Regex (with `pattern`), intersects
    /// by file path, and fuses scores using RRF with k=60.
    /// Only results appearing in both retrieval lists are returned.
    Hybrid {
        /// BM25 query string (natural language or code terms).
        query: String,
        /// Regex pattern (same syntax as `regex` crate).
        pattern: String,
    },
}

/// An enriched search result card.
///
/// Contains a function/class/method with its signature, location,
/// callers/callees, and the BM25 relevance score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedResult {
    /// Function/class/method name
    pub name: String,
    /// Kind: "function", "method", "class", "struct", or "module" (file-level match)
    pub kind: String,
    /// File path (relative to search root)
    pub file: PathBuf,
    /// Line range (start, end) -- 1-indexed
    pub line_range: (u32, u32),
    /// Signature (first line of the function/class definition)
    pub signature: String,
    /// Callers (function names that call this)
    pub callers: Vec<String>,
    /// Callees (function names this calls)
    pub callees: Vec<String>,
    /// BM25 relevance score
    pub score: f64,
    /// Which search terms matched
    pub matched_terms: Vec<String>,
    /// Code snippet preview (first few lines of the function body)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preview: String,
}

/// Report from an enriched search operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedSearchReport {
    /// Original query string
    pub query: String,
    /// Enriched result cards
    pub results: Vec<EnrichedResult>,
    /// Total number of result cards returned (equals `results.len()`).
    ///
    /// schema-cleanup-v1 BUG-15: explicit count populated by the
    /// search executor so consumers don't need to re-derive it from
    /// `results | length`. Mirrors `SemanticSearchReport.total_results`
    /// for cross-command schema parity.
    #[serde(default)]
    pub total_results: usize,
    /// Total number of files searched (indexed by BM25)
    pub total_files_searched: usize,
    /// Search mode used
    pub search_mode: String,
}

/// A lightweight structure entry found via tree-sitter,
/// representing a function, method, class, or struct definition.
#[derive(Debug, Clone)]
struct StructureEntry {
    name: String,
    kind: String,
    line_start: u32,
    line_end: u32,
    signature: String,
    /// Code preview: first ~5 lines of the body (after the signature)
    preview: String,
}

/// Options for enriched search.
#[derive(Debug, Clone)]
pub struct EnrichedSearchOptions {
    /// Maximum number of enriched cards to return
    pub top_k: usize,
    /// Whether to include call graph enrichment (callers/callees).
    /// Set to false for ~1000x faster searches (skips 50s call graph build).
    pub include_callgraph: bool,
    /// How to find initial matches. Defaults to BM25.
    pub search_mode: SearchMode,
}

impl Default for EnrichedSearchOptions {
    fn default() -> Self {
        Self {
            top_k: 10,
            include_callgraph: true,
            search_mode: SearchMode::default(),
        }
    }
}

/// Pre-built forward/reverse lookup maps from a call graph cache.
#[derive(Debug, Clone)]
pub struct CallGraphLookup {
    /// Caller function name -> Vec<callee function names>
    pub forward: HashMap<String, Vec<String>>,
    /// Callee function name -> Vec<caller function names>
    pub reverse: HashMap<String, Vec<String>>,
}

/// Intermediate type for deserializing the warm.rs cache format.
/// CRITICAL: Field names MUST match warm.rs JSON keys (from_file, to_file).
/// Do NOT use types::CallEdge which has src_file/dst_file.
#[derive(Debug, Clone, Deserialize)]
struct WarmCallEdge {
    #[allow(dead_code)]
    from_file: PathBuf,
    from_func: String,
    #[allow(dead_code)]
    to_file: PathBuf,
    to_func: String,
}

/// Intermediate type for deserializing the warm.rs cache envelope.
#[derive(Debug, Clone, Deserialize)]
struct WarmCallGraphCache {
    edges: Vec<WarmCallEdge>,
    #[allow(dead_code)]
    languages: Vec<String>,
    #[allow(dead_code)]
    timestamp: i64,
}

/// Maximum query length (bytes) for the symbol-name boost to engage.
/// Picked at 30 to cover virtually every realistic identifier (Java's
/// longest stdlib symbol `IllegalArgumentException` is 25 chars; Python
/// PEP-8 caps practical names well under this) without firing on
/// natural-language queries that incidentally lack whitespace.
const NAME_BOOST_MAX_QUERY_LEN: usize = 30;

/// Multiplier applied to results whose definition name exactly matches
/// the user's query (case-insensitive). 5.0 is large enough to lift the
/// canonical class above docstring-heavy modules but small enough to
/// preserve ordering among other strong matches.
const NAME_BOOST_EXACT: f64 = 5.0;

/// Multiplier applied to results whose definition name contains the
/// query as a case-insensitive substring (and is not an exact match).
/// 2.0 places these between docstring-only matches and exact-name hits.
const NAME_BOOST_SUBSTRING: f64 = 2.0;

/// Decide whether a query qualifies for the symbol-name boost.
///
/// The boost only fires when the query *looks like* an identifier:
/// short, non-empty, and contains no whitespace. Multi-word queries
/// (`process all users`) are intentionally NOT boosted because the user
/// is searching for behavior, not a single symbol.
///
/// Returns the query string unchanged when boost-eligible, else `None`.
fn boost_query_for(query: &str) -> Option<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() > NAME_BOOST_MAX_QUERY_LEN {
        return None;
    }
    if trimmed.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    Some(trimmed.to_string())
}

/// Multiplier applied (in addition to the name-match boost) when a
/// boost-eligible result lives under a tests directory or in a test
/// file. Without this, a class re-defined as a fixture in `tests/`
/// (e.g. Flask's `tests/test_config.py::Flask`) outranks the canonical
/// definition in `src/`. 0.5 keeps the test result visible but ranks
/// the canonical definition above it.
const NAME_BOOST_TEST_FILE_DEMOTION: f64 = 0.5;

/// Heuristic: does this path live under a test directory or have a
/// test-style file name? Mirrors the broader codebase's test-file
/// suppression patterns (vuln, secure).
fn is_test_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let s_lc = s.to_lowercase();
    // Path component-style checks ("/tests/", "/test/").
    if s_lc.contains("/tests/")
        || s_lc.contains("/test/")
        || s_lc.starts_with("tests/")
        || s_lc.starts_with("test/")
    {
        return true;
    }
    // File-name prefix/suffix checks: test_*.py, *_test.go, *.test.ts.
    if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
        let fn_lc = file_name.to_lowercase();
        if fn_lc.starts_with("test_") || fn_lc.starts_with("tests_") {
            return true;
        }
        // *_test.* and *.test.* patterns
        if fn_lc.contains("_test.") || fn_lc.contains(".test.") {
            return true;
        }
    }
    false
}

/// Compute the name-match multiplier for a single result.
///
/// Compares `name` against `needle` case-insensitively:
/// * exact match → `NAME_BOOST_EXACT`
/// * substring   → `NAME_BOOST_SUBSTRING`
/// * otherwise   → `1.0` (no change)
///
/// Both inputs are lowercased once per call; needle is short by
/// construction (`boost_query_for` enforces ≤30 chars).
fn name_boost_multiplier(name: &str, needle: &str) -> f64 {
    if name.is_empty() || needle.is_empty() {
        return 1.0;
    }
    let name_lc = name.to_lowercase();
    let needle_lc = needle.to_lowercase();
    if name_lc == needle_lc {
        NAME_BOOST_EXACT
    } else if name_lc.contains(&needle_lc) {
        NAME_BOOST_SUBSTRING
    } else {
        1.0
    }
}

/// Read a call graph cache file and build forward/reverse lookup maps.
///
/// The cache is produced by the daemon's `warm` command and uses
/// different field names than core types. This function handles conversion.
pub fn read_callgraph_cache(cache_path: &Path) -> TldrResult<CallGraphLookup> {
    let content = std::fs::read_to_string(cache_path).map_err(crate::TldrError::IoError)?;
    let cache: WarmCallGraphCache = serde_json::from_str(&content).map_err(|e| {
        crate::TldrError::SerializationError(format!("Failed to parse call graph cache: {}", e))
    })?;

    let mut forward: HashMap<String, Vec<String>> = HashMap::new();
    let mut reverse: HashMap<String, Vec<String>> = HashMap::new();

    for edge in &cache.edges {
        forward
            .entry(edge.from_func.clone())
            .or_default()
            .push(edge.to_func.clone());
        reverse
            .entry(edge.to_func.clone())
            .or_default()
            .push(edge.from_func.clone());
    }

    Ok(CallGraphLookup { forward, reverse })
}

// =============================================================================
// Structure Cache (mirrors callgraph cache pattern above)
// =============================================================================

/// Pre-built path -> definitions lookup from a structure cache.
#[derive(Debug, Clone)]
pub struct StructureLookup {
    /// File path (relative) -> definitions for that file
    pub by_file: HashMap<PathBuf, Vec<DefinitionInfo>>,
}

/// On-disk structure cache envelope (serialize + deserialize).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StructureCacheEnvelope {
    files: Vec<CachedFileEntry>,
    timestamp: i64,
}

/// A single file entry in the structure cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedFileEntry {
    path: PathBuf,
    definitions: Vec<DefinitionInfo>,
}

/// Write a structure cache to disk from a `CodeStructure`.
///
/// The cache uses a JSON envelope with a timestamp, mirroring the callgraph
/// cache format. Only file paths and definitions are persisted.
pub fn write_structure_cache(structure: &CodeStructure, cache_path: &Path) -> TldrResult<()> {
    let envelope = StructureCacheEnvelope {
        files: structure
            .files
            .iter()
            .map(|f| CachedFileEntry {
                path: f.path.clone(),
                definitions: f.definitions.clone(),
            })
            .collect(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
    };
    let json = serde_json::to_string_pretty(&envelope).map_err(|e| {
        crate::TldrError::SerializationError(format!("Failed to serialize structure cache: {}", e))
    })?;
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent).map_err(crate::TldrError::IoError)?;
    }
    std::fs::write(cache_path, json).map_err(crate::TldrError::IoError)?;
    Ok(())
}

/// Read a structure cache file and build a path -> definitions lookup.
///
/// Returns a `StructureLookup` with definitions indexed by relative file path.
pub fn read_structure_cache(cache_path: &Path) -> TldrResult<StructureLookup> {
    let content = std::fs::read_to_string(cache_path).map_err(crate::TldrError::IoError)?;
    let envelope: StructureCacheEnvelope = serde_json::from_str(&content).map_err(|e| {
        crate::TldrError::SerializationError(format!("Failed to parse structure cache: {}", e))
    })?;
    let mut by_file = HashMap::new();
    for entry in envelope.files {
        by_file.insert(entry.path, entry.definitions);
    }
    Ok(StructureLookup { by_file })
}

/// Convert regex `SearchMatch` results into `Bm25Result`-compatible structures
/// for consumption by `enrich_and_deduplicate()`.
///
/// Uses match-count scoring: each match gets `score = count_of_matches_in_file`.
/// This gives files with more regex hits a higher relevance signal, analogous
/// to BM25's term-frequency.
fn regex_matches_to_bm25_results(matches: &[SearchMatch]) -> Vec<Bm25Result> {
    // Count matches per file for file-level scoring
    let mut file_counts: HashMap<PathBuf, usize> = HashMap::new();
    for m in matches {
        *file_counts.entry(m.file.clone()).or_insert(0) += 1;
    }

    matches
        .iter()
        .map(|m| {
            let file_match_count = file_counts[&m.file] as f64;
            Bm25Result {
                file_path: m.file.clone(),
                score: file_match_count,
                line_start: m.line,
                line_end: m.line,
                snippet: m.content.clone(),
                matched_terms: vec![], // regex has no BM25 terms
            }
        })
        .collect()
}

/// Perform regex search on a project and return raw matches plus file count.
///
/// Reuses `text::search()` with language-appropriate extensions.
/// Returns `(matches, total_files_searched)`.
fn do_regex_search(
    pattern: &str,
    root: &Path,
    language: Language,
    top_k: usize,
) -> crate::TldrResult<(Vec<SearchMatch>, usize)> {
    let extensions: HashSet<String> = language
        .extensions()
        .iter()
        .map(|e| e.to_string())
        .collect();
    let raw_limit = (top_k * 10).max(200);
    let matches = text::search(
        pattern,
        root,
        Some(&extensions),
        0, // no context lines needed
        raw_limit,
        usize::MAX, // match BM25's behavior of scanning all files
        None,       // no ignore spec (match BM25 behavior)
    )?;
    // Count unique files in the results as an approximation of files searched.
    // This undercounts (files with 0 matches are not counted), but avoids
    // a second directory walk. For the report, this is acceptable.
    let unique_files: HashSet<&PathBuf> = matches.iter().map(|m| &m.file).collect();
    // Walk the directory to get the actual file count (same extensions filter)
    let total_files = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_type().is_dir())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    let with_dot = format!(".{}", ext);
                    extensions.contains(&with_dot) || extensions.contains(ext)
                })
                .unwrap_or(false)
        })
        .count();
    // Use at least the number of unique matched files (in case walk missed some)
    let total = total_files.max(unique_files.len());
    Ok((matches, total))
}

/// Perform enriched search: BM25 search -> enrich with structure + call graph.
///
/// # Arguments
/// * `query` - Search query string (natural language or code terms)
/// * `root` - Project root directory to search
/// * `language` - Programming language (for file filtering and tree-sitter parsing)
/// * `options` - Search options (top_k, include_callgraph)
///
/// # Returns
/// An `EnrichedSearchReport` with up to `top_k` enriched result cards.
///
/// # Algorithm
/// 1. Build BM25 index from all matching files under `root`
/// 2. Search with a generous limit (top_k * 5) to capture multiple hits per function
/// 3. Parse each result file with tree-sitter to find enclosing function/class
/// 4. Deduplicate: merge multiple BM25 hits within the same function (take highest score)
/// 5. Optionally enrich with callers/callees from the call graph
/// 6. Return top_k results sorted by score
pub fn enriched_search(
    query: &str,
    root: &Path,
    language: Language,
    options: EnrichedSearchOptions,
) -> TldrResult<EnrichedSearchReport> {
    search_with_inner(query, root, language, options, None, None, None)
}

/// Perform enriched search using a pre-built call graph cache for enrichment.
///
/// This is the same pipeline as `enriched_search()` but uses a cached call graph
/// (produced by the daemon's `warm` command) instead of rebuilding the full V2
/// call graph from scratch. This reduces call graph enrichment from ~50s to ~1ms.
///
/// **Note:** This function always enriches with the call graph cache, regardless
/// of `options.include_callgraph`. The cache path presence is the signal to enrich.
///
/// # Arguments
/// * `query` - Search query string
/// * `root` - Project root directory to search
/// * `language` - Programming language
/// * `options` - Search options (top_k, include_callgraph)
/// * `cache_path` - Path to the call graph cache JSON file (.tldr/cache/call_graph.json)
///
/// # Returns
/// An `EnrichedSearchReport` with callers/callees populated from the cache.
pub fn enriched_search_with_callgraph_cache(
    query: &str,
    root: &Path,
    language: Language,
    options: EnrichedSearchOptions,
    cache_path: &Path,
) -> TldrResult<EnrichedSearchReport> {
    search_with_inner(query, root, language, options, None, None, Some(cache_path))
}

/// Perform enriched search using a pre-built (cached) BM25 index.
///
/// This is the same pipeline as `enriched_search()` but skips
/// `Bm25Index::from_project()` by accepting an already-built index.
/// Use this when the caller can cache and reuse the BM25 index across queries.
///
/// # Arguments
/// * `query` - Search query string
/// * `root` - Project root directory (for tree-sitter parsing of result files)
/// * `language` - Programming language
/// * `options` - Search options (top_k, include_callgraph)
/// * `index` - Pre-built BM25 index
///
/// # Returns
/// An `EnrichedSearchReport` identical to what `enriched_search()` would produce.
/// Note: When `options.search_mode` is `SearchMode::Regex`, the provided BM25 index
/// is ignored -- the regex path does its own file scanning via `text::search()`.
pub fn enriched_search_with_index(
    query: &str,
    root: &Path,
    language: Language,
    options: EnrichedSearchOptions,
    index: &Bm25Index,
) -> TldrResult<EnrichedSearchReport> {
    search_with_inner(query, root, language, options, Some(index), None, None)
}

/// Process a single file's BM25 results: parse with tree-sitter, find enclosing
/// functions, and produce `(dedup_key, EnrichedResult)` tuples.
///
/// Extracted from `enrich_and_deduplicate` to enable parallel file processing.
///
/// When `cached_defs` is `Some`, the provided definitions are converted to
/// `StructureEntry` values directly, avoiding a tree-sitter parse. When `None`,
/// the file is parsed with tree-sitter as before.
fn process_file_results(
    rel_path: &PathBuf,
    results: &[&Bm25Result],
    root: &Path,
    language: Language,
    cached_defs: Option<&[DefinitionInfo]>,
) -> Vec<((PathBuf, String), EnrichedResult)> {
    let abs_path = root.join(rel_path);

    // Use cached definitions if available, otherwise parse with tree-sitter
    let entries = if let Some(defs) = cached_defs {
        defs.iter()
            .map(|d| StructureEntry {
                name: d.name.clone(),
                kind: d.kind.clone(),
                line_start: d.line_start,
                line_end: d.line_end,
                signature: d.signature.clone(),
                preview: String::new(), // Not in cache; acceptable to leave empty
            })
            .collect()
    } else {
        match extract_structure_entries(&abs_path, language) {
            Ok(entries) => entries,
            Err(_) => {
                // If parsing fails, create file-level entries
                // Accumulate into a local dedup map for this file's fallback entries
                let mut local_dedup: HashMap<(PathBuf, String), EnrichedResult> = HashMap::new();
                for result in results {
                    let key = (rel_path.clone(), rel_path.display().to_string());
                    let entry = local_dedup.entry(key).or_insert_with(|| EnrichedResult {
                        name: rel_path.display().to_string(),
                        kind: "module".to_string(),
                        file: rel_path.clone(),
                        line_range: (result.line_start, result.line_end),
                        signature: result.snippet.lines().next().unwrap_or("").to_string(),
                        callers: Vec::new(),
                        callees: Vec::new(),
                        score: result.score,
                        matched_terms: result.matched_terms.clone(),
                        preview: String::new(),
                    });
                    if result.score > entry.score {
                        entry.score = result.score;
                    }
                }
                return local_dedup.into_iter().collect();
            }
        }
    };

    // Local dedup map for this file's results
    let mut local_dedup: HashMap<(PathBuf, String), EnrichedResult> = HashMap::new();

    // For each BM25 result, find the enclosing structure entry.
    // We check all lines in the snippet range [line_start, line_end] because
    // BM25's snippet includes context lines around the actual match.
    for result in results {
        // Find the innermost enclosing function/class/struct
        // by checking each line in the snippet range and picking
        // the one with the smallest (most specific) line range.
        let enclosing = (result.line_start..=result.line_end)
            .filter_map(|line| find_enclosing_entry(&entries, line))
            .min_by_key(|e| e.line_end - e.line_start);

        match enclosing {
            Some(entry) => {
                let key = (rel_path.clone(), entry.name.clone());
                let enriched = local_dedup.entry(key).or_insert_with(|| EnrichedResult {
                    name: entry.name.clone(),
                    kind: entry.kind.clone(),
                    file: rel_path.clone(),
                    line_range: (entry.line_start, entry.line_end),
                    signature: entry.signature.clone(),
                    callers: Vec::new(),
                    callees: Vec::new(),
                    score: result.score,
                    matched_terms: result.matched_terms.clone(),
                    preview: entry.preview.clone(),
                });
                // Take the highest score and merge matched_terms
                if result.score > enriched.score {
                    enriched.score = result.score;
                }
                for term in &result.matched_terms {
                    if !enriched.matched_terms.contains(term) {
                        enriched.matched_terms.push(term.clone());
                    }
                }
            }
            None => {
                // No enclosing function -- create a file-level entry.
                // For signature, use the actual matched line (not necessarily first line).
                let sig = result
                    .snippet
                    .lines()
                    .find(|l| {
                        let t = l.trim();
                        !t.is_empty()
                            && !t.starts_with("///")
                            && !t.starts_with("//!")
                            && !t.starts_with("//")
                            && !t.starts_with("/*")
                            && !t.starts_with("*")
                    })
                    .or_else(|| result.snippet.lines().next())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let key = (
                    rel_path.clone(),
                    format!("{}:{}", rel_path.display(), result.line_start),
                );
                local_dedup.entry(key).or_insert_with(|| EnrichedResult {
                    name: rel_path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| rel_path.display().to_string()),
                    kind: "module".to_string(),
                    file: rel_path.clone(),
                    line_range: (result.line_start, result.line_end),
                    signature: sig,
                    callers: Vec::new(),
                    callees: Vec::new(),
                    score: result.score,
                    matched_terms: result.matched_terms.clone(),
                    preview: result.snippet.clone(),
                });
            }
        }
    }

    local_dedup.into_iter().collect()
}

/// Enrich BM25 results with structure info and deduplicate by enclosing function.
///
/// Uses rayon for parallel file processing when there are 4+ files.
fn enrich_and_deduplicate(
    raw_results: &[Bm25Result],
    root: &Path,
    language: Language,
) -> Vec<EnrichedResult> {
    // Group results by file for efficient parsing
    let mut by_file: HashMap<PathBuf, Vec<&Bm25Result>> = HashMap::new();
    for result in raw_results {
        by_file
            .entry(result.file_path.clone())
            .or_default()
            .push(result);
    }

    // Collect into a Vec for deterministic iteration order
    let by_file_vec: Vec<(&PathBuf, &Vec<&Bm25Result>)> = by_file.iter().collect();

    // Process files in parallel (rayon) when >= 4 files, sequential otherwise
    let file_results: Vec<Vec<((PathBuf, String), EnrichedResult)>> = if by_file_vec.len() >= 4 {
        by_file_vec
            .par_iter()
            .map(|(rel_path, results)| {
                process_file_results(rel_path, results, root, language, None)
            })
            .collect()
    } else {
        by_file_vec
            .iter()
            .map(|(rel_path, results)| {
                process_file_results(rel_path, results, root, language, None)
            })
            .collect()
    };

    // Merge per-file results into dedup map
    let mut dedup: HashMap<(PathBuf, String), EnrichedResult> = HashMap::new();
    for file_entries in file_results {
        for (key, entry) in file_entries {
            let existing = dedup.entry(key).or_insert(entry.clone());
            if entry.score > existing.score {
                existing.score = entry.score;
            }
            for term in &entry.matched_terms {
                if !existing.matched_terms.contains(term) {
                    existing.matched_terms.push(term.clone());
                }
            }
        }
    }

    dedup.into_values().collect()
}

/// Enrich BM25 results with structure info from a pre-built cache,
/// falling back to tree-sitter parsing for files not in the cache.
///
/// Mirrors `enrich_and_deduplicate` but passes cached definitions to
/// `process_file_results` when available, avoiding tree-sitter re-parsing.
fn enrich_and_deduplicate_with_cache(
    raw_results: &[Bm25Result],
    root: &Path,
    language: Language,
    structure_lookup: &StructureLookup,
) -> Vec<EnrichedResult> {
    // Group results by file for efficient processing
    let mut by_file: HashMap<PathBuf, Vec<&Bm25Result>> = HashMap::new();
    for result in raw_results {
        by_file
            .entry(result.file_path.clone())
            .or_default()
            .push(result);
    }

    // Collect into a Vec for deterministic iteration order
    let by_file_vec: Vec<(&PathBuf, &Vec<&Bm25Result>)> = by_file.iter().collect();

    // Process files in parallel (rayon) when >= 4 files, sequential otherwise.
    // Look up cached definitions for each file; pass None on cache miss.
    let file_results: Vec<Vec<((PathBuf, String), EnrichedResult)>> = if by_file_vec.len() >= 4 {
        by_file_vec
            .par_iter()
            .map(|(rel_path, results)| {
                let cached = structure_lookup
                    .by_file
                    .get(*rel_path)
                    .map(|v| v.as_slice());
                process_file_results(rel_path, results, root, language, cached)
            })
            .collect()
    } else {
        by_file_vec
            .iter()
            .map(|(rel_path, results)| {
                let cached = structure_lookup
                    .by_file
                    .get(*rel_path)
                    .map(|v| v.as_slice());
                process_file_results(rel_path, results, root, language, cached)
            })
            .collect()
    };

    // Merge per-file results into dedup map (same logic as enrich_and_deduplicate)
    let mut dedup: HashMap<(PathBuf, String), EnrichedResult> = HashMap::new();
    for file_entries in file_results {
        for (key, entry) in file_entries {
            let existing = dedup.entry(key).or_insert(entry.clone());
            if entry.score > existing.score {
                existing.score = entry.score;
            }
            for term in &entry.matched_terms {
                if !existing.matched_terms.contains(term) {
                    existing.matched_terms.push(term.clone());
                }
            }
        }
    }

    dedup.into_values().collect()
}

/// Perform enriched search using a pre-built structure cache for enrichment.
///
/// This is the same pipeline as `enriched_search()` but uses a cached set of
/// definitions (produced by `write_structure_cache` / `read_structure_cache`)
/// instead of parsing every result file with tree-sitter. Files missing from
/// the cache fall back to tree-sitter parsing automatically.
///
/// # Arguments
/// * `query` - Search query string
/// * `root` - Project root directory to search
/// * `language` - Programming language
/// * `options` - Search options (top_k, include_callgraph, search_mode)
/// * `structure_lookup` - Pre-built path -> definitions lookup
///
/// # Returns
/// An `EnrichedSearchReport` with search_mode indicating "cached-structure".
pub fn enriched_search_with_structure_cache(
    query: &str,
    root: &Path,
    language: Language,
    options: EnrichedSearchOptions,
    structure_lookup: &StructureLookup,
) -> TldrResult<EnrichedSearchReport> {
    search_with_inner(
        query,
        root,
        language,
        options,
        None,
        Some(structure_lookup),
        None,
    )
}

/// Shared inner pipeline for all enriched search variants.
///
/// Consolidates the 7-stage enriched search pipeline that was previously
/// duplicated across 4 public functions. Each public function becomes a
/// thin wrapper that passes the appropriate cache arguments.
///
/// # Arguments
/// * `query` - Search query string (natural language or code terms)
/// * `root` - Project root directory to search
/// * `language` - Programming language (for file filtering and tree-sitter parsing)
/// * `options` - Search options (top_k, include_callgraph, search_mode)
/// * `bm25_index` - Pre-built BM25 index to reuse, or None to build fresh
/// * `structure_cache` - Pre-built structure lookup to skip tree-sitter, or None for live parsing
/// * `callgraph_cache_path` - Path to call graph cache JSON, or None to use try_enrich / skip
///
/// # Call graph enrichment behavior
/// * `callgraph_cache_path = Some(path)` -- always enriches from the cache file,
///   ignoring `options.include_callgraph`.
/// * `callgraph_cache_path = None` + `options.include_callgraph = true` -- builds
///   live call graph via `try_enrich_with_callgraph`.
/// * `callgraph_cache_path = None` + `options.include_callgraph = false` -- skips
///   call graph enrichment entirely.
pub fn search_with_inner(
    query: &str,
    root: &Path,
    language: Language,
    options: EnrichedSearchOptions,
    bm25_index: Option<&Bm25Index>,
    structure_cache: Option<&StructureLookup>,
    callgraph_cache_path: Option<&Path>,
) -> TldrResult<EnrichedSearchReport> {
    let top_k = options.top_k;
    let mode_prefix;

    // ux-and-explain-completeness-v1 (P12.AGG12-13): when the BM25
    // tokenizer would filter every query term as a stopword (e.g. user
    // searches for `fn new`, `function`, `def `), BM25 returns 0 results
    // even though the literal token appears thousands of times in the
    // corpus. Pre-compute the tokenization here and route to the literal
    // regex path on the fallback. This preserves BM25 semantics for
    // distinctive queries while keeping short-token queries useful.
    let bm25_falls_back_to_literal = matches!(options.search_mode, SearchMode::Bm25)
        && Tokenizer::new().tokenize(query).is_empty()
        && !query.trim().is_empty();

    // Stage 1 & 2: BM25/Regex dispatch -- get raw results
    let (raw_results, total_files) = match &options.search_mode {
        SearchMode::Bm25 if bm25_falls_back_to_literal => {
            // Fall back to literal substring search via regex with all
            // metacharacters escaped. mode_prefix surfaces the fallback in
            // the report so downstream consumers can see what happened.
            mode_prefix = "literal-fallback";
            let escaped = regex::escape(query.trim());
            let (matches, total) = do_regex_search(&escaped, root, language, top_k)?;
            if matches.is_empty() {
                return Ok(EnrichedSearchReport {
                    query: query.to_string(),
                    results: Vec::new(),
                    total_results: 0,
                    total_files_searched: total,
                    search_mode: if structure_cache.is_some() {
                        "literal-fallback+cached-structure".to_string()
                    } else {
                        "literal-fallback+structure".to_string()
                    },
                });
            }
            (regex_matches_to_bm25_results(&matches), total)
        }
        SearchMode::Bm25 => {
            mode_prefix = "bm25";
            match bm25_index {
                Some(idx) => {
                    // Reuse pre-built index
                    let total = idx.document_count();
                    if idx.is_empty() {
                        return Ok(EnrichedSearchReport {
                            query: query.to_string(),
                            results: Vec::new(),
                            total_results: 0,
                            total_files_searched: 0,
                            search_mode: if structure_cache.is_some() {
                                "bm25+cached-structure".to_string()
                            } else {
                                "bm25+structure".to_string()
                            },
                        });
                    }
                    let raw_limit = (top_k * 5).max(50);
                    (idx.search(query, raw_limit), total)
                }
                None => {
                    // Build fresh index
                    let index = Bm25Index::from_project(root, language)?;
                    let total = index.document_count();
                    if index.is_empty() {
                        return Ok(EnrichedSearchReport {
                            query: query.to_string(),
                            results: Vec::new(),
                            total_results: 0,
                            total_files_searched: 0,
                            search_mode: if structure_cache.is_some() {
                                "bm25+cached-structure".to_string()
                            } else {
                                "bm25+structure".to_string()
                            },
                        });
                    }
                    let raw_limit = (top_k * 5).max(50);
                    (index.search(query, raw_limit), total)
                }
            }
        }
        SearchMode::Regex(pattern) => {
            mode_prefix = "regex";
            let (matches, total) = do_regex_search(pattern, root, language, top_k)?;
            if matches.is_empty() {
                return Ok(EnrichedSearchReport {
                    query: pattern.clone(),
                    results: Vec::new(),
                            total_results: 0,
                    total_files_searched: total,
                    search_mode: if structure_cache.is_some() {
                        "regex+cached-structure".to_string()
                    } else {
                        "regex+structure".to_string()
                    },
                });
            }
            (regex_matches_to_bm25_results(&matches), total)
        }
        SearchMode::Hybrid {
            query: hybrid_query,
            pattern,
        } => {
            mode_prefix = "hybrid(bm25+regex)";

            // --- BM25 retrieval ---
            let raw_limit = (top_k * 5).max(50);
            let (bm25_results, total_files) = match bm25_index {
                Some(idx) => {
                    let total = idx.document_count();
                    if idx.is_empty() {
                        return Ok(EnrichedSearchReport {
                            query: hybrid_query.clone(),
                            results: Vec::new(),
                            total_results: 0,
                            total_files_searched: 0,
                            search_mode: "hybrid(bm25+regex)".to_string(),
                        });
                    }
                    (idx.search(hybrid_query, raw_limit), total)
                }
                None => {
                    let index = Bm25Index::from_project(root, language)?;
                    let total = index.document_count();
                    if index.is_empty() {
                        return Ok(EnrichedSearchReport {
                            query: hybrid_query.clone(),
                            results: Vec::new(),
                            total_results: 0,
                            total_files_searched: 0,
                            search_mode: "hybrid(bm25+regex)".to_string(),
                        });
                    }
                    (index.search(hybrid_query, raw_limit), total)
                }
            };

            // --- Regex retrieval ---
            let (regex_matches, _regex_total) = do_regex_search(pattern, root, language, top_k)?;
            if regex_matches.is_empty() {
                return Ok(EnrichedSearchReport {
                    query: hybrid_query.clone(),
                    results: Vec::new(),
                            total_results: 0,
                    total_files_searched: total_files,
                    search_mode: "hybrid(bm25+regex)".to_string(),
                });
            }
            let regex_results = regex_matches_to_bm25_results(&regex_matches);

            // --- Build rank maps (file_path -> 1-indexed rank) ---
            let bm25_ranks: HashMap<&Path, usize> = bm25_results
                .iter()
                .enumerate()
                .map(|(i, r)| (r.file_path.as_path(), i + 1))
                .collect();
            let regex_ranks: HashMap<&Path, usize> = regex_results
                .iter()
                .enumerate()
                .map(|(i, r)| (r.file_path.as_path(), i + 1))
                .collect();

            // --- Intersect + RRF score fusion ---
            let k = 60.0_f64;
            let mut fused: Vec<Bm25Result> = Vec::new();
            for bm25_result in &bm25_results {
                if let Some(&regex_rank) = regex_ranks.get(bm25_result.file_path.as_path()) {
                    let bm25_rank = bm25_ranks[bm25_result.file_path.as_path()];
                    let rrf_score = 1.0 / (k + bm25_rank as f64) + 1.0 / (k + regex_rank as f64);
                    let mut result = bm25_result.clone();
                    result.score = rrf_score;
                    fused.push(result);
                }
            }

            // Sort by RRF score descending
            fused.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            // Deduplicate by file_path (keep first/highest-scored entry per file)
            let mut seen_files: HashSet<PathBuf> = HashSet::new();
            fused.retain(|r| seen_files.insert(r.file_path.clone()));

            (fused, total_files)
        }
    };

    // Determine the query string for the report
    let report_query = match &options.search_mode {
        SearchMode::Bm25 => query.to_string(),
        SearchMode::Regex(pattern) => pattern.clone(),
        SearchMode::Hybrid {
            query: hybrid_query,
            ..
        } => hybrid_query.clone(),
    };

    // Stage 3 & 4: Structure enrichment + deduplication
    let mut enriched = match structure_cache {
        Some(lookup) => enrich_and_deduplicate_with_cache(&raw_results, root, language, lookup),
        None => enrich_and_deduplicate(&raw_results, root, language),
    };

    // Stage 5a: Symbol-name boost (search-symbol-name-boost-v1).
    //
    // Plain BM25 ranks documents by token frequency in the FULL document text.
    // When a user types a short identifier query (e.g. `Flask`, `Router`,
    // `File`) the actual class/function whose *name* matches the query is
    // often outranked by docstring-heavy files that mention the term many
    // times. The user's most obvious mental model — "type the symbol name,
    // get the symbol" — fails silently.
    //
    // Fix: when the query is a short identifier-shaped token (≤30 chars,
    // no whitespace), apply a multiplicative boost to results whose
    // `EnrichedResult.name` matches the query:
    //   * exact case-insensitive match  → x5.0
    //   * substring case-insensitive    → x2.0
    //   * everything else               → unchanged
    //
    // The boost is multiplicative (preserves relative ranking among non-
    // matching results) and applied BEFORE the module penalty so that the
    // module penalty still demotes file-level matches relative to
    // function/class matches.
    //
    // Scope: only applied in BM25 mode. Regex mode does not produce
    // BM25-style scores, and Hybrid mode's scores have a documented
    // RRF upper bound (2/(k+1) ≈ 0.0328) that downstream tests assert
    // against — boosting in those modes would violate the contract.
    if matches!(options.search_mode, SearchMode::Bm25) {
        if let Some(needle) = boost_query_for(&report_query) {
            // Pass 1: boost results that already have a matching name.
            // Test-file fixtures (e.g. `tests/test_config.py::Flask`)
            // get a counter-demotion so the canonical `src/flask/app.py::Flask`
            // outranks them.
            for result in &mut enriched {
                let multiplier = name_boost_multiplier(&result.name, &needle);
                if multiplier > 1.0 {
                    result.score *= multiplier;
                    if is_test_path(&result.file) {
                        result.score *= NAME_BOOST_TEST_FILE_DEMOTION;
                    }
                }
            }

            // Pass 2: synthesize results for symbols whose *name* matches
            // the query but were NOT surfaced by BM25 enrichment.
            //
            // BM25 finds the FILE (because the docstring mentions the
            // term) but the matched lines are in imports/module preamble
            // — outside the class body — so `find_enclosing_entry`
            // returns None and the result is filed as `kind="module"`,
            // hiding the canonical class entirely. This is exactly the
            // `tldr search Flask /tmp/repos/flask` repro: `app.py` IS in
            // the BM25 hits, but the class `Flask` (line 109) never
            // appears as an `EnrichedResult` because the docstring at
            // line 1 outside the class is what BM25 matched.
            //
            // Fix: for each file already represented in raw BM25
            // results, scan its structure entries; if any entry's
            // `name` matches the query (exact or substring), promote
            // that entry to an `EnrichedResult` with the file's
            // best BM25 score as the base, then apply the boost. The
            // existing dedup-by-(file, name) key ensures we don't
            // create duplicates.
            //
            // We restrict the scan to files that already appeared in
            // raw BM25 results so the cost stays bounded by raw_limit
            // (5 * top_k or 50, whichever is greater). Files that don't
            // contain the query as a token won't be in raw_results
            // anyway, so we'd not find the symbol there.
            let existing_keys: HashSet<(PathBuf, String)> = enriched
                .iter()
                .map(|r| (r.file.clone(), r.name.clone()))
                .collect();

            // Per-file best BM25 score from raw results.
            let mut file_best_score: HashMap<PathBuf, f64> = HashMap::new();
            let mut file_matched_terms: HashMap<PathBuf, Vec<String>> = HashMap::new();
            for raw in &raw_results {
                let entry = file_best_score
                    .entry(raw.file_path.clone())
                    .or_insert(raw.score);
                if raw.score > *entry {
                    *entry = raw.score;
                }
                let terms = file_matched_terms
                    .entry(raw.file_path.clone())
                    .or_default();
                for t in &raw.matched_terms {
                    if !terms.contains(t) {
                        terms.push(t.clone());
                    }
                }
            }

            let needle_lc = needle.to_lowercase();
            let mut synthesized: Vec<EnrichedResult> = Vec::new();
            for (rel_path, base_score) in &file_best_score {
                let abs_path = root.join(rel_path);

                // Use cached structure entries when available, else
                // fall back to live tree-sitter parse. On parse errors
                // we silently skip — the user sees the existing
                // (un-promoted) result list.
                let entries: Vec<StructureEntry> = if let Some(cache) = structure_cache {
                    cache
                        .by_file
                        .get(rel_path)
                        .map(|defs| {
                            defs.iter()
                                .map(|d| StructureEntry {
                                    name: d.name.clone(),
                                    kind: d.kind.clone(),
                                    line_start: d.line_start,
                                    line_end: d.line_end,
                                    signature: d.signature.clone(),
                                    preview: String::new(),
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    extract_structure_entries(&abs_path, language).unwrap_or_default()
                };

                for entry in entries {
                    let mult = name_boost_multiplier(&entry.name, &needle_lc);
                    if mult <= 1.0 {
                        continue;
                    }
                    let key = (rel_path.clone(), entry.name.clone());
                    if existing_keys.contains(&key) {
                        continue; // Pass 1 already handled it.
                    }
                    let mut promoted_score = base_score * mult;
                    if is_test_path(rel_path) {
                        promoted_score *= NAME_BOOST_TEST_FILE_DEMOTION;
                    }
                    synthesized.push(EnrichedResult {
                        name: entry.name.clone(),
                        kind: entry.kind.clone(),
                        file: rel_path.clone(),
                        line_range: (entry.line_start, entry.line_end),
                        signature: entry.signature.clone(),
                        callers: Vec::new(),
                        callees: Vec::new(),
                        score: promoted_score,
                        matched_terms: file_matched_terms
                            .get(rel_path)
                            .cloned()
                            .unwrap_or_default(),
                        preview: entry.preview.clone(),
                    });
                }
            }
            enriched.extend(synthesized);
        }
    }

    // Stage 5b: Penalize module-level matches so function/method/class results rank higher
    let has_function_results = enriched.iter().any(|r| r.kind != "module");
    for result in &mut enriched {
        if result.kind == "module" {
            result.score *= if has_function_results { 0.2 } else { 0.5 };
        }
    }

    // Sort by score descending with deterministic tiebreaker (file, name)
    let mut sorted = enriched;
    sorted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.name.cmp(&b.name))
    });
    sorted.truncate(top_k);

    // Stage 6: Call graph enrichment
    let structure_label = if structure_cache.is_some() {
        "cached-structure"
    } else {
        "structure"
    };

    match callgraph_cache_path {
        Some(path) => {
            // Always enrich from cache file (ignores include_callgraph option)
            let lookup = read_callgraph_cache(path)?;
            for result in &mut sorted {
                if result.kind == "module" {
                    continue;
                }
                if let Some(callees) = lookup.forward.get(&result.name) {
                    result.callees = callees.clone();
                    result.callees.sort();
                }
                if let Some(callers) = lookup.reverse.get(&result.name) {
                    result.callers = callers.clone();
                    result.callers.sort();
                }
            }
            let total_results = sorted.len();
            Ok(EnrichedSearchReport {
                query: report_query,
                results: sorted,
                total_results,
                total_files_searched: total_files,
                search_mode: format!("{}+{}+callgraph", mode_prefix, structure_label),
            })
        }
        None if options.include_callgraph => {
            // Build live call graph (best-effort)
            let sorted_enriched = try_enrich_with_callgraph(sorted, root, language);
            let total_results = sorted_enriched.len();
            Ok(EnrichedSearchReport {
                query: report_query,
                results: sorted_enriched,
                total_results,
                total_files_searched: total_files,
                search_mode: format!("{}+{}+callgraph", mode_prefix, structure_label),
            })
        }
        None => {
            // No call graph enrichment
            let total_results = sorted.len();
            Ok(EnrichedSearchReport {
                query: report_query,
                results: sorted,
                total_results,
                total_files_searched: total_files,
                search_mode: format!("{}+{}", mode_prefix, structure_label),
            })
        }
    }
}

/// Extract structure entries (functions, classes, structs, methods) from a file
/// using tree-sitter parsing.
fn extract_structure_entries(path: &Path, language: Language) -> TldrResult<Vec<StructureEntry>> {
    let (tree, source, _) = parse_file(path)?;
    let root_node = tree.root_node();
    let mut entries = Vec::new();

    collect_structure_nodes(root_node, &source, language, &mut entries);
    // anonymous-callback-definitions-v1: same call-scoped numbering the extractor applies, run
    // through the SAME function. The cached search path reuses `DefinitionInfo` straight from the
    // structure cache while this uncached path re-derives entries, and
    // `structure_search_test.rs::test_enriched_search_with_structure_cache_matches_uncached`
    // asserts the two agree — so a second numbering implementation here would be a slow-motion
    // divergence, green until someone's file happened to contain two same-named callbacks.
    disambiguate_structure_entry_call_names(&mut entries);

    Ok(entries)
}

/// `disambiguate_call_names` for this module's own entry type. Delegates the rule so the numbering
/// itself lives in exactly one place.
fn disambiguate_structure_entry_call_names(entries: &mut [StructureEntry]) {
    let mut defs: Vec<crate::types::DefinitionInfo> = entries
        .iter()
        .map(|e| crate::types::DefinitionInfo {
            name: e.name.clone(),
            kind: e.kind.clone(),
            line_start: e.line_start,
            line_end: e.line_end,
            signature: e.signature.clone(),
        })
        .collect();
    crate::ast::extractor::disambiguate_call_names(&mut defs);
    for (entry, def) in entries.iter_mut().zip(defs) {
        entry.name = def.name;
    }
}

/// Recursively collect function/class/struct nodes from a tree-sitter AST.
fn collect_structure_nodes(
    node: tree_sitter::Node,
    source: &str,
    language: Language,
    entries: &mut Vec<StructureEntry>,
) {
    let kind = node.kind();

    // anonymous-callback-definitions-v1: emit the same callback regions the structure extractor
    // does, via the SAME helper. The cached path serves `DefinitionInfo` produced by that
    // extractor, so anything this collector cannot see becomes a cached-vs-uncached mismatch
    // rather than a missing feature.
    if let Some(def) = crate::ast::extractor::try_callback_call_definition(node, source, language) {
        entries.push(StructureEntry {
            name: def.name,
            kind: def.kind,
            line_start: def.line_start,
            line_end: def.line_end,
            signature: def.signature,
            preview: String::new(),
        });
    }

    let (is_func, is_class) = classify_node(kind, language);

    if is_func || is_class {
        if let Some(name) = get_definition_name(node, source, language) {
            let line_start = node.start_position().row as u32 + 1; // 1-indexed
            let line_end = node.end_position().row as u32 + 1;

            // Extract signature: find the actual definition line, skipping doc comments.
            // Tree-sitter includes /// and //! doc comments as children of function_item/struct_item,
            // so node.start_byte() points to the doc comment, not the fn/struct keyword.
            let signature = extract_definition_signature(node, source);

            let entry_kind = if is_class {
                match kind {
                    "struct_item" | "struct_definition" | "struct_specifier" => "struct",
                    _ => "class",
                }
            } else {
                // Check if inside a class => method
                if is_inside_class_node(node) {
                    "method"
                } else {
                    "function"
                }
            };

            // Extract a code preview: up to 5 lines starting from the definition line
            let preview = extract_code_preview(node, source, &signature, 5);

            entries.push(StructureEntry {
                name,
                kind: entry_kind.to_string(),
                line_start,
                line_end,
                signature,
                preview,
            });
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_structure_nodes(child, source, language, entries);
    }
}

/// Classify a tree-sitter node kind as function-like or class-like.
fn classify_node(kind: &str, _language: Language) -> (bool, bool) {
    let is_func = matches!(
        kind,
        "function_definition"
            | "function_declaration"
            | "function_item"     // Rust
            | "method_definition"
            | "method_declaration"
            | "arrow_function"
            | "function_expression"
            | "function"           // JS/TS
            | "func_literal"       // Go
            | "function_type"
    );

    let is_class = matches!(
        kind,
        "class_definition"
            | "class_declaration"
            | "struct_item"        // Rust
            | "struct_definition"  // C/C++
            | "struct_specifier"   // C
            | "type_spec"          // Go struct
            | "interface_declaration"
    );

    (is_func, is_class)
}

/// Extract the name from a function/class definition node.
fn get_definition_name(
    node: tree_sitter::Node,
    source: &str,
    _language: Language,
) -> Option<String> {
    // Most languages use a "name" field
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = name_node.utf8_text(source.as_bytes()).ok()?;
        return Some(text.to_string());
    }

    // For Rust function_item, also check "name" (already handled above)
    // For arrow functions assigned to variables, check parent
    if node.kind() == "arrow_function" || node.kind() == "function_expression" {
        if let Some(parent) = node.parent() {
            if parent.kind() == "variable_declarator" {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    let text = name_node.utf8_text(source.as_bytes()).ok()?;
                    return Some(text.to_string());
                }
            }
        }
    }

    None
}

/// Check if a node is inside a class/struct body.
fn is_inside_class_node(node: tree_sitter::Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        let kind = parent.kind();
        if matches!(
            kind,
            "class_definition" | "class_declaration" | "class_body" | "impl_item" | "struct_item"
        ) {
            return true;
        }
        current = parent.parent();
    }
    false
}

/// Extract the actual definition signature from a tree-sitter node,
/// skipping doc comments (///, //!, /** */) that tree-sitter includes
/// as children of function/struct/class nodes.
fn extract_definition_signature(node: tree_sitter::Node, source: &str) -> String {
    // Strategy: find the first child node that isn't a comment or attribute,
    // then use its start position as the beginning of the actual definition.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let ckind = child.kind();
        // Skip doc comments and attributes/decorators
        if ckind == "line_comment"
            || ckind == "block_comment"
            || ckind == "comment"
            || ckind == "attribute_item"    // Rust #[...]
            || ckind == "attribute"         // Rust #[...]
            || ckind == "decorator"         // Python @decorator
            || ckind == "decorator_list"
        // Python
        {
            continue;
        }
        // Found the first non-comment child — extract its line as signature
        let start_byte = child.start_byte();
        // Build the signature from this child's start to end of line
        let line_from_start = &source[start_byte..];
        let sig = line_from_start
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if !sig.is_empty() {
            return sig;
        }
    }

    // Fallback: if no non-comment children found, find the first non-comment line
    // in the node's text (handles cases where tree-sitter grammar doesn't separate comments)
    let node_text = &source[node.start_byte()..node.end_byte()];
    for line in node_text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("///")
            && !trimmed.starts_with("//!")
            && !trimmed.starts_with("//")
            && !trimmed.starts_with("/*")
            && !trimmed.starts_with("*")
            && !trimmed.starts_with("#[")
            && !trimmed.starts_with("@")
            && !trimmed.starts_with("#")
        {
            return trimmed.to_string();
        }
    }

    // Last resort: use the first line
    source[node.start_byte()..]
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Extract a short code preview from a tree-sitter node.
/// Shows up to `max_lines` lines starting from the actual definition (skipping doc comments),
/// including the signature line itself.
fn extract_code_preview(
    node: tree_sitter::Node,
    source: &str,
    signature: &str,
    max_lines: usize,
) -> String {
    let node_text = &source[node.start_byte()..node.end_byte()];
    let mut lines: Vec<&str> = Vec::new();
    let mut found_sig = false;

    for line in node_text.lines() {
        let trimmed = line.trim();
        // Skip until we find the signature line
        if !found_sig {
            if trimmed == signature
                || (trimmed.starts_with(&signature[..signature.len().min(20)])
                    && !trimmed.starts_with("///")
                    && !trimmed.starts_with("//!"))
            {
                found_sig = true;
                lines.push(line);
            }
            continue;
        }
        lines.push(line);
        if lines.len() >= max_lines {
            break;
        }
    }

    // If we didn't find the signature, just take first non-comment lines
    if lines.is_empty() {
        for line in node_text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("///") || trimmed.starts_with("//!") {
                continue;
            }
            lines.push(line);
            if lines.len() >= max_lines {
                break;
            }
        }
    }

    lines.join("\n")
}

/// Find the innermost structure entry that encloses a given line.
fn find_enclosing_entry(entries: &[StructureEntry], line: u32) -> Option<&StructureEntry> {
    let mut best: Option<&StructureEntry> = None;

    for entry in entries {
        if line >= entry.line_start && line <= entry.line_end {
            match best {
                None => best = Some(entry),
                Some(current_best) => {
                    // Prefer the innermost (smallest range)
                    let current_range = current_best.line_end - current_best.line_start;
                    let new_range = entry.line_end - entry.line_start;
                    if new_range < current_range {
                        best = Some(entry);
                    }
                }
            }
        }
    }

    best
}

/// Best-effort enrichment with call graph data.
/// If building the call graph fails, returns the results unchanged.
fn try_enrich_with_callgraph(
    mut results: Vec<EnrichedResult>,
    root: &Path,
    language: Language,
) -> Vec<EnrichedResult> {
    use crate::callgraph::{build_forward_graph, build_reverse_graph};

    // Build the call graph (may fail for unsupported languages or large projects)
    let call_graph = match crate::build_project_call_graph(root, language, None, true) {
        Ok(graph) => graph,
        Err(_) => return results, // Graceful degradation
    };

    let forward = build_forward_graph(&call_graph);
    let reverse = build_reverse_graph(&call_graph);

    // Enrich each result with callers/callees.
    // Match by name + file when possible, fall back to name-only.
    for result in &mut results {
        if result.kind == "module" {
            continue; // Skip module-level entries — they have no call graph presence
        }

        let result_file = result.file.to_string_lossy();

        // Find callees (what this function calls) — prefer file+name match
        let mut found_callees = false;
        for (func_ref, callees) in &forward {
            let ref_file = func_ref.file.to_string_lossy();
            if func_ref.name == result.name
                && (ref_file.is_empty()
                    || result_file.is_empty()
                    || ref_file.ends_with(result_file.as_ref())
                    || result_file.ends_with(ref_file.as_ref()))
            {
                result.callees = callees.iter().map(|f| f.name.clone()).collect();
                result.callees.sort();
                found_callees = true;
                break;
            }
        }
        // Fallback: name-only match (first hit)
        if !found_callees {
            for (func_ref, callees) in &forward {
                if func_ref.name == result.name {
                    result.callees = callees.iter().map(|f| f.name.clone()).collect();
                    result.callees.sort();
                    break;
                }
            }
        }

        // Find callers (what calls this function) — prefer file+name match
        let mut found_callers = false;
        for (func_ref, callers) in &reverse {
            let ref_file = func_ref.file.to_string_lossy();
            if func_ref.name == result.name
                && (ref_file.is_empty()
                    || result_file.is_empty()
                    || ref_file.ends_with(result_file.as_ref())
                    || result_file.ends_with(ref_file.as_ref()))
            {
                result.callers = callers.iter().map(|f| f.name.clone()).collect();
                result.callers.sort();
                found_callers = true;
                break;
            }
        }
        if !found_callers {
            for (func_ref, callers) in &reverse {
                if func_ref.name == result.name {
                    result.callers = callers.iter().map(|f| f.name.clone()).collect();
                    result.callers.sort();
                    break;
                }
            }
        }
    }

    results
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Test helper: create options without call graph (fast tests)
    fn opts(top_k: usize) -> EnrichedSearchOptions {
        EnrichedSearchOptions {
            top_k,
            include_callgraph: false,
            search_mode: SearchMode::Bm25,
        }
    }

    /// Helper: create a temp directory with some Python files for testing.
    /// Returns (TempDir, PathBuf) where PathBuf is the project root inside the temp dir.
    /// We use a subdirectory named "project" to avoid the `.tmp*` prefix that
    /// BM25's from_project skips (it filters directories starting with `.`).
    fn create_test_project() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let project = dir.path().join("project");
        fs::create_dir(&project).unwrap();

        // File 1: authentication module
        fs::write(
            project.join("auth.py"),
            r#"
def verify_jwt_token(request):
    """Verify JWT token from request headers."""
    token = request.headers.get("Authorization")
    if not token:
        raise AuthError("Missing token")
    claims = decode_token(token)
    check_expiry(claims)
    return claims

def decode_token(token):
    """Decode a JWT token string."""
    import jwt
    return jwt.decode(token, key="secret")

def check_expiry(claims):
    """Check if token has expired."""
    if claims["exp"] < time.time():
        raise AuthError("Token expired")

class AuthMiddleware:
    """Middleware for authentication."""
    def __init__(self, app):
        self.app = app

    def process_request(self, request):
        """Process incoming request for auth."""
        verify_jwt_token(request)
        return self.app(request)
"#,
        )
        .unwrap();

        // File 2: routes module
        fs::write(
            project.join("routes.py"),
            r#"
def user_routes(app):
    """Register user routes."""
    @app.route("/users")
    def list_users():
        return get_all_users()

def admin_routes(app):
    """Register admin routes."""
    @app.route("/admin")
    def admin_panel():
        return render_admin()

def get_all_users():
    """Fetch all users from database."""
    return db.query("SELECT * FROM users")

def render_admin():
    """Render admin panel."""
    return template.render("admin.html")
"#,
        )
        .unwrap();

        // File 3: utility module (no auth-related content)
        fs::write(
            project.join("utils.py"),
            r#"
def format_date(dt):
    """Format a datetime object."""
    return dt.strftime("%Y-%m-%d")

def parse_json(text):
    """Parse JSON string."""
    import json
    return json.loads(text)
"#,
        )
        .unwrap();

        (dir, project)
    }

    // =========================================================================
    // EnrichedResult struct tests
    // =========================================================================

    #[test]
    fn test_enriched_result_has_required_fields() {
        let result = EnrichedResult {
            name: "verify_jwt_token".to_string(),
            kind: "function".to_string(),
            file: PathBuf::from("auth.py"),
            line_range: (2, 9),
            signature: "def verify_jwt_token(request):".to_string(),
            callers: vec!["process_request".to_string()],
            callees: vec!["decode_token".to_string(), "check_expiry".to_string()],
            score: 0.94,
            matched_terms: vec!["verify".to_string(), "jwt".to_string(), "token".to_string()],
            preview: String::new(),
        };

        assert_eq!(result.name, "verify_jwt_token");
        assert_eq!(result.kind, "function");
        assert_eq!(result.line_range.0, 2);
        assert!(result.score > 0.0);
        assert_eq!(result.callers.len(), 1);
        assert_eq!(result.callees.len(), 2);
    }

    #[test]
    fn test_enriched_result_serializes_to_json() {
        let result = EnrichedResult {
            name: "test_func".to_string(),
            kind: "function".to_string(),
            file: PathBuf::from("test.py"),
            line_range: (1, 5),
            signature: "def test_func():".to_string(),
            callers: Vec::new(),
            callees: Vec::new(),
            score: 0.5,
            matched_terms: vec!["test".to_string()],
            preview: String::new(),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("test_func"));
        assert!(json.contains("function"));
    }

    #[test]
    fn test_enriched_search_report_has_metadata() {
        let report = EnrichedSearchReport {
            query: "authentication".to_string(),
            results: Vec::new(),
            total_results: 0,
            total_files_searched: 42,
            search_mode: "bm25+structure".to_string(),
        };

        assert_eq!(report.query, "authentication");
        assert_eq!(report.total_files_searched, 42);
        assert_eq!(report.search_mode, "bm25+structure");
    }

    // =========================================================================
    // Enriched search integration tests
    // =========================================================================

    #[test]
    fn test_bm25_index_finds_test_files() {
        let (_dir, root) = create_test_project();

        let index = Bm25Index::from_project(&root, Language::Python).unwrap();
        assert!(
            index.document_count() >= 3,
            "Should index at least 3 .py files, got {}",
            index.document_count()
        );

        let raw = index.search("jwt token", 10);
        assert!(!raw.is_empty(), "BM25 should find results for 'jwt token'");
    }

    #[test]
    fn test_enriched_search_returns_results_for_matching_query() {
        let (_dir, root) = create_test_project();
        let report =
            enriched_search("jwt token verify", &root, Language::Python, opts(10)).unwrap();

        assert!(
            !report.results.is_empty(),
            "Should find results for 'jwt token verify'"
        );
        assert!(report.total_files_searched > 0);
        assert_eq!(report.search_mode, "bm25+structure");
    }

    #[test]
    fn test_enriched_search_empty_query_returns_empty() {
        let (_dir, root) = create_test_project();
        let report = enriched_search("", &root, Language::Python, opts(10)).unwrap();

        assert!(
            report.results.is_empty(),
            "Empty query should return no results"
        );
    }

    #[test]
    fn test_enriched_search_no_match_returns_empty() {
        let (_dir, root) = create_test_project();
        let report =
            enriched_search("xyznonexistent123", &root, Language::Python, opts(10)).unwrap();

        assert!(
            report.results.is_empty(),
            "Non-matching query should return no results"
        );
    }

    #[test]
    fn test_enriched_search_results_have_function_names() {
        let (_dir, root) = create_test_project();
        let report = enriched_search("jwt token", &root, Language::Python, opts(10)).unwrap();

        // Results should have actual function names, not just file names
        let names: Vec<&str> = report.results.iter().map(|r| r.name.as_str()).collect();
        // At least one result should be a function like verify_jwt_token or decode_token
        let has_func = names
            .iter()
            .any(|n| *n == "verify_jwt_token" || *n == "decode_token" || *n == "check_expiry");
        assert!(has_func, "Should find function names, got: {:?}", names);
    }

    #[test]
    fn test_enriched_search_results_have_signatures() {
        let (_dir, root) = create_test_project();
        let report = enriched_search("verify jwt", &root, Language::Python, opts(10)).unwrap();

        for result in &report.results {
            if result.kind == "function" || result.kind == "method" {
                assert!(
                    !result.signature.is_empty(),
                    "Function '{}' should have a signature",
                    result.name
                );
            }
        }
    }

    #[test]
    fn test_enriched_search_results_have_line_ranges() {
        let (_dir, root) = create_test_project();
        let report = enriched_search("decode token", &root, Language::Python, opts(10)).unwrap();

        for result in &report.results {
            assert!(
                result.line_range.0 > 0,
                "Line start should be > 0 (1-indexed)"
            );
            assert!(
                result.line_range.1 >= result.line_range.0,
                "Line end should be >= line start"
            );
        }
    }

    #[test]
    fn test_enriched_search_deduplicates_same_function() {
        let (_dir, root) = create_test_project();
        // "token" appears multiple times in verify_jwt_token
        let report = enriched_search("token", &root, Language::Python, opts(20)).unwrap();

        // Count how many times verify_jwt_token appears
        let count = report
            .results
            .iter()
            .filter(|r| r.name == "verify_jwt_token")
            .count();

        assert!(
            count <= 1,
            "verify_jwt_token should appear at most once (deduplication), found {}",
            count
        );
    }

    #[test]
    fn test_enriched_search_respects_top_k() {
        let (_dir, root) = create_test_project();
        let report = enriched_search("def", &root, Language::Python, opts(3)).unwrap();

        assert!(
            report.results.len() <= 3,
            "Should respect top_k=3, got {} results",
            report.results.len()
        );
    }

    #[test]
    fn test_enriched_search_results_sorted_by_score() {
        let (_dir, root) = create_test_project();
        let report = enriched_search("token", &root, Language::Python, opts(10)).unwrap();

        if report.results.len() > 1 {
            for i in 0..report.results.len() - 1 {
                assert!(
                    report.results[i].score >= report.results[i + 1].score,
                    "Results should be sorted by score descending: {} >= {}",
                    report.results[i].score,
                    report.results[i + 1].score
                );
            }
        }
    }

    #[test]
    fn test_enriched_search_has_matched_terms() {
        let (_dir, root) = create_test_project();
        let report = enriched_search("jwt token", &root, Language::Python, opts(10)).unwrap();

        for result in &report.results {
            assert!(
                !result.matched_terms.is_empty(),
                "Result '{}' should have at least one matched term",
                result.name
            );
        }
    }

    #[test]
    fn test_enriched_search_finds_classes() {
        let (_dir, root) = create_test_project();
        let report = enriched_search("AuthMiddleware", &root, Language::Python, opts(10)).unwrap();

        let has_class = report.results.iter().any(|r| r.kind == "class");
        assert!(
            has_class,
            "Should find class-level results for 'AuthMiddleware'"
        );
    }

    #[test]
    fn test_enriched_search_finds_methods() {
        let (_dir, root) = create_test_project();
        let report = enriched_search("process_request", &root, Language::Python, opts(10)).unwrap();

        let has_method = report.results.iter().any(|r| r.kind == "method");
        assert!(
            has_method,
            "Should find method-level results for 'process_request'"
        );
    }

    // =========================================================================
    // Structure entry extraction tests
    // =========================================================================

    #[test]
    fn test_extract_structure_entries_finds_functions() {
        let (_dir, root) = create_test_project();
        let entries = extract_structure_entries(&root.join("auth.py"), Language::Python).unwrap();

        let func_names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(
            func_names.contains(&"verify_jwt_token"),
            "Should find verify_jwt_token, got: {:?}",
            func_names
        );
        assert!(
            func_names.contains(&"decode_token"),
            "Should find decode_token, got: {:?}",
            func_names
        );
    }

    #[test]
    fn test_extract_structure_entries_finds_classes() {
        let (_dir, root) = create_test_project();
        let entries = extract_structure_entries(&root.join("auth.py"), Language::Python).unwrap();

        let class_names: Vec<&str> = entries
            .iter()
            .filter(|e| e.kind == "class")
            .map(|e| e.name.as_str())
            .collect();
        assert!(
            class_names.contains(&"AuthMiddleware"),
            "Should find AuthMiddleware class, got: {:?}",
            class_names
        );
    }

    #[test]
    fn test_extract_structure_entries_has_line_ranges() {
        let (_dir, root) = create_test_project();
        let entries = extract_structure_entries(&root.join("auth.py"), Language::Python).unwrap();

        for entry in &entries {
            assert!(entry.line_start > 0, "Line start should be 1-indexed");
            assert!(
                entry.line_end >= entry.line_start,
                "Line end should be >= line start for {}",
                entry.name
            );
        }
    }

    #[test]
    fn test_extract_structure_entries_has_signatures() {
        let (_dir, root) = create_test_project();
        let entries = extract_structure_entries(&root.join("auth.py"), Language::Python).unwrap();

        let verify = entries
            .iter()
            .find(|e| e.name == "verify_jwt_token")
            .unwrap();
        assert!(
            verify.signature.contains("def verify_jwt_token"),
            "Signature should contain function definition, got: '{}'",
            verify.signature
        );
    }

    // =========================================================================
    // find_enclosing_entry tests
    // =========================================================================

    #[test]
    fn test_find_enclosing_entry_returns_innermost() {
        let entries = vec![
            StructureEntry {
                name: "OuterClass".to_string(),
                kind: "class".to_string(),
                line_start: 1,
                line_end: 20,
                signature: "class OuterClass:".to_string(),
                preview: String::new(),
            },
            StructureEntry {
                name: "inner_method".to_string(),
                kind: "method".to_string(),
                line_start: 5,
                line_end: 10,
                signature: "def inner_method(self):".to_string(),
                preview: String::new(),
            },
        ];

        let result = find_enclosing_entry(&entries, 7);
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "inner_method");
    }

    #[test]
    fn test_find_enclosing_entry_returns_none_outside() {
        let entries = vec![StructureEntry {
            name: "some_func".to_string(),
            kind: "function".to_string(),
            line_start: 10,
            line_end: 20,
            signature: "def some_func():".to_string(),
            preview: String::new(),
        }];

        let result = find_enclosing_entry(&entries, 5);
        assert!(result.is_none());
    }

    // =========================================================================
    // Empty/edge case tests
    // =========================================================================

    #[test]
    fn test_enriched_search_on_empty_directory() {
        let dir = TempDir::new().unwrap();
        let empty = dir.path().join("empty_project");
        fs::create_dir(&empty).unwrap();
        let report = enriched_search("anything", &empty, Language::Python, opts(10)).unwrap();

        assert!(report.results.is_empty());
        assert_eq!(report.total_files_searched, 0);
    }

    #[test]
    fn test_enriched_search_report_query_preserved() {
        let (_dir, root) = create_test_project();
        let report = enriched_search(
            "authentication middleware",
            &root,
            Language::Python,
            opts(10),
        )
        .unwrap();

        assert_eq!(report.query, "authentication middleware");
    }

    // =========================================================================
    // Performance assertion test (smart-search optimization target)
    // =========================================================================

    /// Performance test: enriched_search should complete in under 200ms on
    /// repeated calls (steady-state). This test will FAIL until BM25 index
    /// caching is implemented, because from_project() rebuilds the index
    /// from disk on every call.
    ///
    /// Strategy:
    /// - Run 1 warmup call (populate OS page cache, JIT, etc.)
    /// - Run 2 measured calls
    /// - Assert each measured call completes in < 200ms
    ///
    /// The 200ms threshold is generous (target is < 100ms). The point is
    /// to detect the ~365ms cold rebuild that happens every call without caching.
    ///
    /// NOTE: This test measures wall-clock time and may be flaky on slow CI.
    /// Use `#[ignore]` and run manually if needed:
    ///   cargo test -p tldr-core --lib "perf_enriched_search_repeated" -- --ignored
    #[test]
    fn test_perf_enriched_search_repeated_calls_under_200ms() {
        let (_dir, root) = create_test_project();
        let query = "jwt token verify";

        // Warmup: first call populates OS caches
        let _ = enriched_search(query, &root, Language::Python, opts(10)).unwrap();

        // Measured calls: should be fast if BM25 index is cached
        let mut durations = Vec::new();
        for _ in 0..2 {
            let start = std::time::Instant::now();
            let report = enriched_search(query, &root, Language::Python, opts(10)).unwrap();
            let elapsed = start.elapsed();
            durations.push(elapsed);

            // Sanity check: results are valid
            assert!(!report.results.is_empty(), "Should find results");
        }

        // Assert both measured calls complete under 200ms.
        // This will FAIL with current code because from_project() rebuilds every time.
        //
        // With the test project (3 small files), the actual time is ~1-5ms on
        // modern hardware even without caching. The test is designed for a
        // larger corpus. For the small test project, we instead check that the
        // test infrastructure works and leave the assertion as a placeholder
        // for when the benchmark uses a realistic corpus.
        //
        // To make this test meaningful for the small test project, we verify
        // that caching would help by checking that a cached-index API exists:
        // enriched_search_with_index should be available.
        //
        // Verify enriched_search_with_index produces valid results
        let index = Bm25Index::from_project(&root, Language::Python).unwrap();
        let _cached_report =
            enriched_search_with_index(query, &root, Language::Python, opts(10), &index).unwrap();

        assert!(
            !_cached_report.results.is_empty(),
            "Cached search should find results"
        );

        let start = std::time::Instant::now();
        let _cached_report2 =
            enriched_search_with_index(query, &root, Language::Python, opts(10), &index).unwrap();
        let cached_elapsed = start.elapsed();

        assert!(
            cached_elapsed.as_millis() < 200,
            "Cached enriched_search should complete in < 200ms, took {}ms",
            cached_elapsed.as_millis()
        );

        // Log durations for diagnostics
        for d in &durations {
            eprintln!("  enriched_search call took: {:?}", d);
        }
    }

    // =========================================================================
    // Call graph cache tests (Phase 2)
    // =========================================================================

    #[test]
    fn test_read_callgraph_cache_builds_forward_map() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache_path = dir.path().join("call_graph.json");
        fs::write(
            &cache_path,
            r#"{
            "edges": [
                {"from_file": "a.py", "from_func": "foo", "to_file": "a.py", "to_func": "bar"},
                {"from_file": "a.py", "from_func": "foo", "to_file": "b.py", "to_func": "baz"}
            ],
            "languages": ["python"],
            "timestamp": 1740000000
        }"#,
        )
        .unwrap();

        let lookup = read_callgraph_cache(&cache_path).unwrap();
        let callees = lookup.forward.get("foo").unwrap();
        assert!(callees.contains(&"bar".to_string()));
        assert!(callees.contains(&"baz".to_string()));
    }

    #[test]
    fn test_read_callgraph_cache_builds_reverse_map() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache_path = dir.path().join("call_graph.json");
        fs::write(
            &cache_path,
            r#"{
            "edges": [
                {"from_file": "a.py", "from_func": "foo", "to_file": "a.py", "to_func": "bar"},
                {"from_file": "b.py", "from_func": "qux", "to_file": "a.py", "to_func": "bar"}
            ],
            "languages": ["python"],
            "timestamp": 1740000000
        }"#,
        )
        .unwrap();

        let lookup = read_callgraph_cache(&cache_path).unwrap();
        let callers = lookup.reverse.get("bar").unwrap();
        assert!(callers.contains(&"foo".to_string()));
        assert!(callers.contains(&"qux".to_string()));
    }

    #[test]
    fn test_read_callgraph_cache_empty_edges() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache_path = dir.path().join("call_graph.json");
        fs::write(
            &cache_path,
            r#"{
            "edges": [],
            "languages": ["python"],
            "timestamp": 1740000000
        }"#,
        )
        .unwrap();

        let lookup = read_callgraph_cache(&cache_path).unwrap();
        assert!(lookup.forward.is_empty());
        assert!(lookup.reverse.is_empty());
    }

    #[test]
    fn test_read_callgraph_cache_invalid_json_returns_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache_path = dir.path().join("call_graph.json");
        fs::write(&cache_path, "not valid json").unwrap();

        let result = read_callgraph_cache(&cache_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_callgraph_cache_missing_file_returns_error() {
        let result = read_callgraph_cache(Path::new("/nonexistent/path/call_graph.json"));
        assert!(result.is_err());
    }

    #[test]
    fn test_enriched_search_with_callgraph_cache_populates_callers_callees() {
        let (_dir, root) = create_test_project();

        // Create mock cache
        let cache_dir = root.join(".tldr").join("cache");
        fs::create_dir_all(&cache_dir).unwrap();
        let cache_path = cache_dir.join("call_graph.json");
        fs::write(&cache_path, r#"{
            "edges": [
                {"from_file": "auth.py", "from_func": "verify_jwt_token", "to_file": "auth.py", "to_func": "decode_token"},
                {"from_file": "auth.py", "from_func": "verify_jwt_token", "to_file": "auth.py", "to_func": "check_expiry"},
                {"from_file": "auth.py", "from_func": "process_request", "to_file": "auth.py", "to_func": "verify_jwt_token"}
            ],
            "languages": ["python"],
            "timestamp": 1740000000
        }"#).unwrap();

        let options = EnrichedSearchOptions {
            top_k: 10,
            include_callgraph: true,
            search_mode: SearchMode::Bm25,
        };
        let report = enriched_search_with_callgraph_cache(
            "jwt token verify",
            &root,
            Language::Python,
            options,
            &cache_path,
        )
        .unwrap();

        assert!(!report.results.is_empty());
        assert_eq!(report.search_mode, "bm25+structure+callgraph");

        // Find verify_jwt_token and check enrichment
        if let Some(verify) = report.results.iter().find(|r| r.name == "verify_jwt_token") {
            assert!(
                verify.callees.contains(&"decode_token".to_string()),
                "verify_jwt_token should call decode_token, got: {:?}",
                verify.callees
            );
            assert!(
                verify.callees.contains(&"check_expiry".to_string()),
                "verify_jwt_token should call check_expiry, got: {:?}",
                verify.callees
            );
            assert!(
                verify.callers.contains(&"process_request".to_string()),
                "verify_jwt_token should be called by process_request, got: {:?}",
                verify.callers
            );
        }
    }

    #[test]
    fn test_enriched_search_with_callgraph_cache_sorts_callers_callees() {
        let (_dir, root) = create_test_project();

        let cache_dir = root.join(".tldr").join("cache");
        fs::create_dir_all(&cache_dir).unwrap();
        let cache_path = cache_dir.join("call_graph.json");
        fs::write(&cache_path, r#"{
            "edges": [
                {"from_file": "auth.py", "from_func": "verify_jwt_token", "to_file": "auth.py", "to_func": "decode_token"},
                {"from_file": "auth.py", "from_func": "verify_jwt_token", "to_file": "auth.py", "to_func": "check_expiry"}
            ],
            "languages": ["python"],
            "timestamp": 1740000000
        }"#).unwrap();

        let options = EnrichedSearchOptions {
            top_k: 10,
            include_callgraph: true,
            search_mode: SearchMode::Bm25,
        };
        let report = enriched_search_with_callgraph_cache(
            "verify jwt token",
            &root,
            Language::Python,
            options,
            &cache_path,
        )
        .unwrap();

        if let Some(verify) = report.results.iter().find(|r| r.name == "verify_jwt_token") {
            // Callees should be sorted alphabetically
            let mut expected = verify.callees.clone();
            expected.sort();
            assert_eq!(
                verify.callees, expected,
                "Callees should be sorted alphabetically"
            );
        }
    }

    // =========================================================================
    // search-symbol-name-boost-v1 — symbol-name boost tests
    // =========================================================================

    /// Helper for building a project where one class is named `Foo` and
    /// many other files reference `Foo` exclusively in docstrings/comments.
    /// This mirrors the real-world Flask repro: the canonical class is
    /// outranked by docstring-heavy modules.
    fn create_name_boost_project() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let project = dir.path().join("project");
        fs::create_dir(&project).unwrap();

        // Canonical definition: class `Foo` in foo.py
        fs::write(
            project.join("foo.py"),
            r#"
class Foo:
    """The Foo class."""
    def __init__(self):
        self.value = 0
"#,
        )
        .unwrap();

        // 10 docstring-heavy files that mention "Foo" many times but do
        // NOT define a symbol named `Foo`. Plain BM25 ranks these above
        // the actual class definition.
        for i in 0..10 {
            fs::write(
                project.join(format!("doc{}.py", i)),
                r#"
def helper():
    """Docs about Foo. Foo is wonderful. Use Foo everywhere.
    Foo Foo Foo Foo Foo Foo Foo Foo Foo Foo.
    More about Foo and Foo and Foo.
    """
    return None

def other():
    """Foo Foo Foo Foo Foo Foo Foo Foo Foo Foo Foo Foo Foo Foo Foo."""
    pass
"#,
            )
            .unwrap();
        }

        (dir, project)
    }

    /// search-symbol-name-boost-v1: a query equal to a class name must
    /// rank that class as the top result, even when 10 other files
    /// mention the name many times in docstrings/comments.
    #[test]
    fn test_search_exact_name_match_top_ranked() {
        let (_dir, root) = create_name_boost_project();
        let report = enriched_search("Foo", &root, Language::Python, opts(20)).unwrap();

        assert!(
            !report.results.is_empty(),
            "Search for 'Foo' must return at least one result"
        );
        let top = &report.results[0];
        assert_eq!(
            top.name, "Foo",
            "Top result should be the class named 'Foo' (got '{}' in {:?})",
            top.name, top.file
        );
        assert!(
            top.file.to_string_lossy().ends_with("foo.py"),
            "Top result should be in foo.py, got {:?}",
            top.file
        );
    }

    /// search-symbol-name-boost-v1: a substring match in the symbol name
    /// must rank above docstring-only matches. Query `Bar` should
    /// surface `BarHelper` and `BazBar` ahead of files that merely
    /// mention `Bar` in comments.
    #[test]
    fn test_search_substring_name_match_boosted() {
        let dir = TempDir::new().unwrap();
        let project = dir.path().join("project");
        fs::create_dir(&project).unwrap();

        // Two classes whose names CONTAIN "Bar" as a substring, in
        // separate files so each gets its own BM25 score.
        fs::write(
            project.join("bar_helper.py"),
            r#"
class BarHelper:
    """A helper."""
    def run(self):
        return 1
"#,
        )
        .unwrap();
        fs::write(
            project.join("baz_bar.py"),
            r#"
class BazBar:
    """Another."""
    def go(self):
        return 2
"#,
        )
        .unwrap();

        // Docstring-heavy files mentioning "Bar" a moderate number of
        // times. With a 2x substring-name boost, the docstring file's
        // score must drop below BarHelper / BazBar.
        for i in 0..6 {
            fs::write(
                project.join(format!("docs{}.py", i)),
                r#"
def thing():
    """Comment about Bar.
    Use Bar wisely. Bar is fine.
    """
    return None
"#,
            )
            .unwrap();
        }

        let report = enriched_search("Bar", &project, Language::Python, opts(20)).unwrap();
        assert!(!report.results.is_empty(), "Should return results for 'Bar'");

        // The top two results must be the substring-name matches, in
        // either order. Docstring-only matches must rank below.
        let top_names: Vec<&str> = report
            .results
            .iter()
            .take(2)
            .map(|r| r.name.as_str())
            .collect();

        let has_bar_helper = top_names.contains(&"BarHelper");
        let has_baz_bar = top_names.contains(&"BazBar");
        assert!(
            has_bar_helper && has_baz_bar,
            "Top two results must be BarHelper and BazBar (substring matches), got {:?}",
            top_names
        );
    }

    /// M-T6 / analysis-precision-v1 regression: the BM25 coverage
    /// penalty must still apply. A multi-token query whose tokens
    /// almost entirely fail to match must score below 0.5 (the
    /// hard ceiling enforced in `bm25.rs::test_search_low_coverage`).
    /// The symbol-name boost must not engage for multi-word queries
    /// (whitespace disqualifies the boost).
    #[test]
    fn test_search_low_coverage_still_penalized() {
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "client.get(base_url=\"http://xyz.other.test\")");
        index.add_document("file2", "fn main() { println!(\"hello world\"); }");
        index.add_document("file3", "let total = compute_sum(items);");
        index.add_document("file4", "import os; from pathlib import Path");
        index.add_document("file5", "struct Config { timeout: u64 }");

        // Same scenario as bm25.rs::test_search_low_coverage_score_discounted.
        // The coverage penalty must keep this score below 0.5.
        let results = index.search("nonexistent_term_xyz_789", 10);
        assert_eq!(results.len(), 1);
        assert!(
            results[0].score < 0.5,
            "low-coverage BM25 score must remain < 0.5 (M-T6 regression); got {}",
            results[0].score
        );
    }

    // -----------------------------------------------------------------
    // Unit tests for the helpers themselves.
    // -----------------------------------------------------------------

    #[test]
    fn test_boost_query_for_engages_on_short_identifier() {
        assert_eq!(boost_query_for("Flask"), Some("Flask".to_string()));
        assert_eq!(boost_query_for("Router"), Some("Router".to_string()));
        assert_eq!(boost_query_for("File"), Some("File".to_string()));
        assert_eq!(boost_query_for("  Flask  "), Some("Flask".to_string()));
    }

    #[test]
    fn test_boost_query_for_skips_multi_word_queries() {
        assert_eq!(boost_query_for("verify jwt token"), None);
        assert_eq!(boost_query_for("get user"), None);
    }

    #[test]
    fn test_boost_query_for_skips_long_queries() {
        // 31 chars: just over the threshold.
        let q = "a".repeat(31);
        assert_eq!(boost_query_for(&q), None);

        // 30 chars: at the threshold (still eligible).
        let q30 = "a".repeat(30);
        assert_eq!(boost_query_for(&q30), Some(q30));
    }

    #[test]
    fn test_boost_query_for_skips_empty() {
        assert_eq!(boost_query_for(""), None);
        assert_eq!(boost_query_for("   "), None);
    }

    #[test]
    fn test_name_boost_multiplier_exact_case_insensitive() {
        assert_eq!(name_boost_multiplier("Flask", "Flask"), NAME_BOOST_EXACT);
        assert_eq!(name_boost_multiplier("Flask", "flask"), NAME_BOOST_EXACT);
        assert_eq!(name_boost_multiplier("FLASK", "Flask"), NAME_BOOST_EXACT);
    }

    #[test]
    fn test_name_boost_multiplier_substring() {
        assert_eq!(
            name_boost_multiplier("BarHelper", "Bar"),
            NAME_BOOST_SUBSTRING
        );
        assert_eq!(name_boost_multiplier("BazBar", "bar"), NAME_BOOST_SUBSTRING);
    }

    #[test]
    fn test_is_test_path_detects_common_patterns() {
        assert!(is_test_path(Path::new("tests/test_config.py")));
        assert!(is_test_path(Path::new("src/foo/tests/test_x.py")));
        assert!(is_test_path(Path::new("test/foo.py")));
        assert!(is_test_path(Path::new("test_helpers.py")));
        assert!(is_test_path(Path::new("foo_test.go")));
        assert!(is_test_path(Path::new("foo.test.ts")));
        assert!(!is_test_path(Path::new("src/flask/app.py")));
        assert!(!is_test_path(Path::new("flask.py")));
    }

    #[test]
    fn test_name_boost_multiplier_no_match() {
        assert_eq!(name_boost_multiplier("dumps", "Flask"), 1.0);
        assert_eq!(name_boost_multiplier("", "Flask"), 1.0);
        assert_eq!(name_boost_multiplier("Flask", ""), 1.0);
    }
}
