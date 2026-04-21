# Premortem Analysis - Pass 2/3
## Semantic Search Module for tldr-rs

**Date:** 2026-02-03  
**Pass Focus:** User Experience, Accuracy, Edge Cases, Compatibility, Concurrency  
**Assumption:** The project has FAILED. What went wrong?

---

## 1. User Experience Failures

### 1.1 Confusing Score Interpretation

**Failure Scenario:** Users see scores like `0.52` and don't know if that's good or bad. They filter with `--threshold 0.8` expecting "good matches" but get zero results, then lower to `0.3` and get garbage.

**Likelihood:** HIGH  
**Impact:** MAJOR

**Mitigation:**
```rust
/// In SemanticSearchReport, add interpretive guidance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSearchReport {
    // ... existing fields ...
    
    /// Human-readable score interpretation guide
    pub score_guide: ScoreGuide,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreGuide {
    /// Suggested threshold for "strong match"
    pub strong_threshold: f64,  // 0.75+
    /// Suggested threshold for "relevant"
    pub relevant_threshold: f64,  // 0.55+
    /// Warning if all scores are low
    pub low_score_warning: Option<String>,
}

// In text output, show:
// "Results (8 matches):
//  Score guide: >=0.75 = strong match, >=0.55 = relevant, <0.55 = weak
// 
//  1. src/config.rs:parse_config (score: 0.89) [STRONG]
//  2. src/loader.rs:load_config (score: 0.62) [RELEVANT]
//  3. src/util.rs:init (score: 0.41) [WEAK]"
```

**Add to spec Section 4.1:** Include `score_guide` in output and text formatting showing score interpretation badges.

---

### 1.2 Silent Truncation of Long Functions

**Failure Scenario:** A 500-line function exceeds the 512 token context limit. The embedding only represents the first ~80 lines. User searches for logic in line 400, gets no match, thinks the tool is broken.

**Likelihood:** HIGH  
**Impact:** CRITICAL

**Mitigation:**
```rust
/// Track truncation in chunk metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChunk {
    // ... existing fields ...
    
    /// True if content was truncated to fit model context
    pub truncated: bool,
    
    /// Original character count before truncation
    pub original_length: usize,
}

/// In output, warn about truncated functions
impl SemanticSearchReport {
    pub fn truncation_warnings(&self) -> Vec<String> {
        // Return warnings for any truncated results
    }
}

// Text output should show:
// "WARNING: 3 functions were truncated (>512 tokens). 
//  Consider using --model m-long for better coverage:
//  - src/big_module.rs:process_all (truncated from 15KB)
//  - ..."
```

**Add to spec Section 2.1:** Add `truncated: bool` and `original_length: usize` to CodeChunk. Add truncation warnings to report output.

---

### 1.3 Opaque Model Download Experience

**Failure Scenario:** First-time user runs `tldr semantic "error handling"` and the terminal hangs for 30+ seconds with no output while the 110MB model downloads. User thinks it's frozen and Ctrl+C's.

**Likelihood:** HIGH  
**Impact:** MAJOR

**Mitigation:**
```rust
/// In embedder.rs, always show download progress
impl Embedder {
    pub fn new(model: EmbeddingModel) -> TldrResult<Self> {
        // Check if model exists locally first
        if !model.is_cached() {
            eprintln!(
                "Downloading {} embedding model (~{}MB)...",
                model.name(),
                model.size_mb()
            );
            eprintln!("This is a one-time download. Future runs will be instant.");
        }
        
        // Use fastembed's progress callback if available
        // ...
    }
}

// Add --offline flag to fail fast without download
#[arg(long)]
pub offline: bool,
```

**Add to spec Section 6.3:** Require progress message before any model download. Add `--offline` flag to all semantic commands.

---

### 1.4 Meaningless Results for Non-Code Queries

**Failure Scenario:** User searches `"fix the bug"` or `"TODO"` - gets results with scores ~0.4-0.5 that look plausible but are essentially random. User doesn't realize semantic search works best for conceptual queries.

**Likelihood:** MEDIUM  
**Impact:** MAJOR

**Mitigation:**
```rust
/// Detect low-quality queries and warn
fn analyze_query_quality(query: &str) -> QueryQuality {
    let words: Vec<&str> = query.split_whitespace().collect();
    
    // Flag queries that are too short
    if words.len() < 2 {
        return QueryQuality::TooShort;
    }
    
    // Flag queries that are just keywords (should use grep instead)
    let keyword_patterns = ["TODO", "FIXME", "BUG", "error"];
    if words.iter().all(|w| keyword_patterns.contains(w) || w.len() < 3) {
        return QueryQuality::UseGrepInstead;
    }
    
    QueryQuality::Good
}

// In output:
// "HINT: Your query 'TODO' may work better with `tldr search TODO` (keyword search)
//  Semantic search excels at conceptual queries like 'handle authentication errors'"
```

**Add to spec Section 4.1:** Add query quality analysis with hints suggesting keyword search when appropriate.

---

### 1.5 No Indication of Index Staleness

**Failure Scenario:** User adds new functions to codebase, runs semantic search, wonders why new code isn't found. Cache has old embeddings, user doesn't know to use `--no-cache`.

**Likelihood:** MEDIUM  
**Impact:** MAJOR

**Mitigation:**
```rust
/// In SemanticSearchReport, add cache freshness info
pub struct SemanticSearchReport {
    // ... existing fields ...
    
    /// Files that were re-indexed (changed since cache)
    pub files_reindexed: usize,
    
    /// Files loaded from cache
    pub files_from_cache: usize,
    
    /// Oldest cache entry age (if using cache)
    pub oldest_cache_age_hours: Option<u64>,
}

// In text output:
// "Index: 150 chunks (142 cached, 8 re-indexed)
//  Cache age: oldest entry is 72 hours old
//  TIP: Use --no-cache to force full re-index"
```

**Add to spec Section 4.1:** Report cache statistics including age of oldest entry and count of re-indexed files.

---

## 2. Accuracy Failures

### 2.1 Cross-Language Semantic Mismatch

**Failure Scenario:** User searches `"parse JSON"` in a mixed Python/Rust codebase. The Python `json.loads()` wrapper ranks higher than the Rust `serde_json::from_str()` implementation because the embedding model was trained more on Python.

**Likelihood:** MEDIUM  
**Impact:** MAJOR

**Mitigation:**
```rust
/// Support language-aware search boost
#[derive(Debug, Clone)]
pub struct SearchOptions {
    // ... existing fields ...
    
    /// Boost factor for specific language (1.0 = no boost)
    pub language_boost: Option<(Language, f64)>,
}

// Example: --lang-boost rust:1.2
// This multiplies Rust results' scores by 1.2

// In similarity.rs:
fn apply_language_boost(
    results: &mut [SemanticSearchResult],
    boost: Option<(Language, f64)>,
) {
    if let Some((lang, factor)) = boost {
        for r in results.iter_mut() {
            if r.language == lang {
                r.score *= factor;
                r.boosted = true;
            }
        }
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    }
}
```

**Add to spec Section 2.4:** Add optional `language_boost` to SearchOptions. Add `--lang-boost` CLI flag.

---

### 2.2 Docstring-Dominated Embeddings

**Failure Scenario:** Functions with verbose docstrings match queries based on docstring content, not actual implementation. User searches `"sort by timestamp"` and gets functions whose docstrings mention sorting but actually do something else.

**Likelihood:** MEDIUM  
**Impact:** MAJOR

**Mitigation:**
```rust
/// Chunk options for docstring handling
pub struct ChunkOptions {
    // ... existing fields ...
    
    /// How to handle docstrings
    pub docstring_mode: DocstringMode,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum DocstringMode {
    /// Include docstrings (default)
    #[default]
    Include,
    /// Exclude docstrings entirely
    Exclude,
    /// Give docstrings lower weight (separate embedding, lower score contribution)
    Downweight,
}

// Alternative: Generate TWO embeddings per function
// - One for code only
// - One for docstring only
// Then combine with configurable weight
```

**Add to spec Section 2.3:** Add `DocstringMode` enum and `docstring_mode` to ChunkOptions. Default to Include but document trade-offs.

---

### 2.3 Variable Name Over-Influence

**Failure Scenario:** User searches `"authentication"` and gets functions that just happen to have a variable named `auth` but do unrelated work (e.g., `let auth = config.get("auth");` in a logging function).

**Likelihood:** MEDIUM  
**Impact:** MINOR

**Mitigation:**
```rust
/// Consider adding code normalization before embedding
fn normalize_code_for_embedding(code: &str, language: Language) -> String {
    // Option 1: Replace variable names with generic placeholders
    // Option 2: Extract only structural patterns (risky, loses semantics)
    // Option 3: Document this limitation and suggest using function-name queries
    
    // For now, document the limitation:
    // "Semantic search matches against full code including variable names.
    //  For precise identifier matching, use `tldr search --pattern 'auth'`"
    code.to_string()
}
```

**Add to spec Section 8.3:** Document that variable names influence embeddings. Suggest keyword search for identifier matching.

---

### 2.4 Boilerplate Code Pollution

**Failure Scenario:** Large codebase has many similar boilerplate functions (constructors, getters, init methods). These dominate search results because they all have similar embeddings and match generic queries.

**Likelihood:** MEDIUM  
**Impact:** MAJOR

**Mitigation:**
```rust
/// Filter out boilerplate patterns
pub struct SearchOptions {
    // ... existing fields ...
    
    /// Exclude common boilerplate patterns
    pub exclude_boilerplate: bool,
}

/// Boilerplate detection heuristics
fn is_likely_boilerplate(chunk: &CodeChunk) -> bool {
    let name = chunk.function_name.as_deref().unwrap_or("");
    
    // Common boilerplate function names
    let boilerplate_names = [
        "new", "default", "clone", "from", "into",
        "__init__", "__str__", "__repr__",
        "get_", "set_", "is_", "has_",
        "toString", "hashCode", "equals",
    ];
    
    boilerplate_names.iter().any(|b| name.starts_with(b) || name == *b)
        || chunk.content.lines().count() < 5  // Very short functions
}

// In search, optionally filter:
// "Tip: Use --no-boilerplate to hide 23 constructor/getter results"
```

**Add to spec Section 2.4:** Add `exclude_boilerplate: bool` to SearchOptions with configurable patterns.

---

## 3. Edge Case Failures

### 3.1 Empty Repository / No Code Files

**Failure Scenario:** User runs `tldr semantic` on a directory with only markdown, configs, and no source code. Gets cryptic error or silently returns empty results.

**Likelihood:** MEDIUM  
**Impact:** MINOR

**Mitigation:**
```rust
/// Provide helpful error for no-code scenarios
pub fn build(root: &Path, ...) -> TldrResult<Self> {
    let chunks = chunk_code(root, chunk_options)?;
    
    if chunks.is_empty() {
        // Check WHY it's empty
        let file_count = count_files(root);
        let code_extensions = [".py", ".rs", ".ts", ".go"];
        let has_code_files = find_files_with_extensions(root, &code_extensions).count() > 0;
        
        if file_count == 0 {
            return Err(TldrError::NoChunksFound {
                path: root.to_path_buf(),
                hint: "Directory is empty".into(),
            });
        } else if !has_code_files {
            return Err(TldrError::NoChunksFound {
                path: root.to_path_buf(),
                hint: format!(
                    "Found {} files but none with supported extensions ({:?}). \
                     Use --lang to specify language.",
                    file_count, code_extensions
                ),
            });
        } else {
            return Err(TldrError::NoChunksFound {
                path: root.to_path_buf(),
                hint: "Files found but no functions extracted. Check for parse errors.".into(),
            });
        }
    }
    // ...
}
```

**Add to spec Section 6.1:** Enhance `NoChunksFound` error with diagnostic hints.

---

### 3.2 Single-File Project

**Failure Scenario:** User runs `tldr similar src/main.rs` on a project with only one file. Gets either self-match or empty results. User confused about what "similar" means with no comparisons.

**Likelihood:** LOW  
**Impact:** MINOR

**Mitigation:**
```rust
/// Special handling for single-file projects
impl SemanticIndex {
    pub fn find_similar(&self, chunk: &CodeChunk, options: SearchOptions) -> TldrResult<SimilarityReport> {
        let candidates = self.chunks_excluding(chunk, options.exclude_self);
        
        if candidates.is_empty() {
            return Ok(SimilarityReport {
                similar: vec![],
                note: Some("No other functions to compare. Use --include-self to see self-similarity.".into()),
                // ...
            });
        }
        // ...
    }
}
```

**Add to spec Section 3.3:** Handle single-file/single-function gracefully with explanatory message.

---

### 3.3 Monorepo with 100K+ Functions

**Failure Scenario:** User runs semantic search on a massive monorepo. Index building takes 10+ minutes, runs out of memory, or search becomes unusably slow.

**Likelihood:** MEDIUM  
**Impact:** CRITICAL

**Mitigation:**
```rust
/// Add hard limits and user feedback
pub const MAX_INDEX_SIZE: usize = 100_000;
pub const WARNING_INDEX_SIZE: usize = 50_000;

impl SemanticIndex {
    pub fn build(root: &Path, ...) -> TldrResult<Self> {
        let chunks = chunk_code(root, chunk_options)?;
        
        if chunks.len() > MAX_INDEX_SIZE {
            return Err(TldrError::IndexTooLarge {
                count: chunks.len(),
                limit: MAX_INDEX_SIZE,
                suggestion: format!(
                    "Index exceeds {} chunks. Suggestions:\n\
                     - Use --path to target a subdirectory\n\
                     - Use --lang to filter by language\n\
                     - Use --exclude to skip directories (e.g., vendor/, node_modules/)",
                    MAX_INDEX_SIZE
                ),
            });
        }
        
        if chunks.len() > WARNING_INDEX_SIZE {
            eprintln!(
                "Warning: Large index ({} chunks). Search may be slow. \
                 Consider narrowing scope with --path or --exclude.",
                chunks.len()
            );
        }
        // ...
    }
}

// Add --exclude patterns to CLI
#[arg(long, value_delimiter = ',')]
pub exclude: Vec<String>,  // e.g., --exclude vendor,node_modules,test
```

**Add to spec Section 8.4:** Add `MAX_INDEX_SIZE`, `WARNING_INDEX_SIZE` constants. Add `--exclude` CLI flag. Add `IndexTooLarge` error type.

---

### 3.4 Files with Syntax Errors

**Failure Scenario:** Codebase has a file with a syntax error (WIP code). Tree-sitter fails to parse, entire file is skipped silently. User's search misses important function.

**Likelihood:** MEDIUM  
**Impact:** MAJOR

**Mitigation:**
```rust
/// Track parse failures in report
pub struct EmbedReport {
    // ... existing fields ...
    
    /// Files that failed to parse (with errors)
    pub parse_failures: Vec<ParseFailure>,
}

pub struct ParseFailure {
    pub file_path: PathBuf,
    pub error: String,
    pub line: Option<u32>,
}

// In output:
// "Indexed 145 chunks from 23 files
//  WARNING: 2 files skipped due to parse errors:
//  - src/wip.py:42: SyntaxError: unexpected indent
//  - src/broken.rs:10: expected `;`"
```

**Add to spec Section 3.2:** Track and report parse failures. Add `parse_failures` to EmbedReport.

---

### 3.5 Unicode/Non-ASCII Code

**Failure Scenario:** Codebase has functions with non-ASCII names (e.g., `calculate_日期()` or comments in Chinese). Embedding model handles them poorly, search accuracy degrades.

**Likelihood:** LOW  
**Impact:** MINOR

**Mitigation:**
```rust
/// Document Unicode handling
// In spec: "The Snowflake Arctic models are trained primarily on English text.
// Non-ASCII content (variable names, comments in other languages) may have
// reduced search accuracy. Consider using keyword search for non-English queries."

/// Optionally strip non-ASCII for embedding
pub struct ChunkOptions {
    // ... existing fields ...
    
    /// Normalize to ASCII for embedding (experimental)
    pub ascii_only: bool,
}
```

**Add to spec Section 5.1:** Document Unicode limitations of Arctic models. Add note about reduced accuracy for non-English content.

---

## 4. Compatibility Failures

### 4.1 ONNX Runtime Version Mismatch

**Failure Scenario:** User has `onnxruntime 1.16` installed system-wide, but fastembed requires `1.18`. Build succeeds but runtime crashes with cryptic C++ errors.

**Likelihood:** MEDIUM  
**Impact:** CRITICAL

**Mitigation:**
```toml
# In Cargo.toml, pin ONNX runtime version strictly
[dependencies]
fastembed = { version = "5.8", features = ["static-onnx"] }
# OR
ort = { version = "2.0", features = ["static"] }  # Static linking avoids system conflicts
```

```rust
/// Add version check at startup
fn check_onnx_compatibility() -> TldrResult<()> {
    // If using dynamic linking, verify version
    if let Some(version) = ort::sys::version() {
        if version < MIN_ONNX_VERSION {
            return Err(TldrError::IncompatibleRuntime {
                found: version,
                required: MIN_ONNX_VERSION,
            });
        }
    }
    Ok(())
}
```

**Add to spec Section 10.1:** Recommend static ONNX linking. Add `IncompatibleRuntime` error type.

---

### 4.2 Rust Version Incompatibility

**Failure Scenario:** User on Rust 1.70 tries to build tldr-rs but fastembed requires 1.75+ for certain features. Build fails with confusing type errors.

**Likelihood:** LOW  
**Impact:** MAJOR

**Mitigation:**
```toml
# In Cargo.toml
[package]
rust-version = "1.75"  # Explicit MSRV

# In lib.rs
#[cfg(not(rust_version = "1.75"))]
compile_error!("Semantic search requires Rust 1.75 or later");
```

**Add to spec Section 10:** Document minimum Rust version (1.75+) for semantic module.

---

### 4.3 Platform-Specific ONNX Issues

**Failure Scenario:** Works on macOS/Linux but fails on Windows due to missing ONNX runtime redistributable or ARM-specific issues on M1/M2 Macs.

**Likelihood:** MEDIUM  
**Impact:** MAJOR

**Mitigation:**
```rust
/// Add platform-specific fallback
#[cfg(target_os = "windows")]
fn setup_onnx_runtime() -> TldrResult<()> {
    // Check for Visual C++ redistributable
    // Provide helpful error if missing
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn setup_onnx_runtime() -> TldrResult<()> {
    // M1/M2 specific checks
    // May need to use CPU execution provider instead of CoreML
}

// Graceful degradation:
// "Semantic search unavailable on this platform. Using keyword search as fallback."
```

**Add to spec Section 6.3:** Document platform requirements. Add graceful degradation when ONNX unavailable.

---

### 4.4 Disk Space Exhaustion During Model Download

**Failure Scenario:** User's home directory is nearly full. Model download (110MB) starts, partially downloads, then fails. Leaves corrupted partial file that causes future failures.

**Likelihood:** LOW  
**Impact:** MAJOR

**Mitigation:**
```rust
/// Atomic model download with space check
fn download_model(model: EmbeddingModel) -> TldrResult<PathBuf> {
    let required_mb = model.size_mb() + 50;  // Buffer for extraction
    let available_mb = get_available_space_mb(&model_cache_dir())?;
    
    if available_mb < required_mb {
        return Err(TldrError::InsufficientDiskSpace {
            required_mb,
            available_mb,
            cache_dir: model_cache_dir(),
        });
    }
    
    // Download to temp file first
    let temp_path = cache_dir.join(format!("{}.downloading", model.name()));
    download_to(&temp_path)?;
    
    // Atomic rename on success
    std::fs::rename(&temp_path, &final_path)?;
    
    Ok(final_path)
}
```

**Add to spec Section 6.2:** Check disk space before download. Use atomic download pattern (temp file + rename).

---

## 5. Concurrency Failures

### 5.1 Concurrent Cache Writes Corrupt JSON

**Failure Scenario:** User runs `tldr semantic` in two terminals simultaneously. Both try to write to the same cache JSON file. Result: corrupted JSON, all cached embeddings lost.

**Likelihood:** HIGH  
**Impact:** CRITICAL

**Mitigation:**
```rust
/// Use file locking for cache writes
impl EmbeddingCache {
    pub fn flush(&mut self) -> TldrResult<()> {
        use fs2::FileExt;
        
        let lock_path = self.path.with_extension("lock");
        let lock_file = File::create(&lock_path)?;
        
        // Exclusive lock for writing
        lock_file.lock_exclusive()?;
        
        // Write to temp file first (atomic)
        let temp_path = self.path.with_extension("tmp");
        let temp_file = File::create(&temp_path)?;
        serde_json::to_writer_pretty(&temp_file, &self.entries)?;
        temp_file.sync_all()?;
        
        // Atomic rename
        std::fs::rename(&temp_path, &self.path)?;
        
        // Release lock
        lock_file.unlock()?;
        
        Ok(())
    }
    
    pub fn open(config: CacheConfig) -> TldrResult<Self> {
        use fs2::FileExt;
        
        let lock_path = config.cache_path().with_extension("lock");
        let lock_file = File::open(&lock_path).or_else(|_| File::create(&lock_path))?;
        
        // Shared lock for reading
        lock_file.lock_shared()?;
        
        let entries = if config.cache_path().exists() {
            let file = File::open(&config.cache_path())?;
            serde_json::from_reader(file)?
        } else {
            HashMap::new()
        };
        
        lock_file.unlock()?;
        
        Ok(Self { entries, .. })
    }
}
```

**Add to spec Section 5.3:** Require file locking for cache operations. Use `fs2` crate for cross-platform locks.

**Add to Cargo.toml:**
```toml
fs2 = "0.4"
```

---

### 5.2 Race Condition in Cache Invalidation

**Failure Scenario:** Process A reads file, computes hash, checks cache. Process B modifies file. Process A writes embedding with stale hash. Future lookups use wrong embedding.

**Likelihood:** MEDIUM  
**Impact:** MAJOR

**Mitigation:**
```rust
/// Include file mtime in cache key for extra safety
struct CacheKey {
    file_path: PathBuf,
    content_hash: String,
    model: EmbeddingModel,
    mtime: SystemTime,  // Additional check
}

impl EmbeddingCache {
    pub fn get(&self, chunk: &CodeChunk, model: EmbeddingModel) -> Option<Vec<f32>> {
        let key = self.make_key(chunk, model);
        
        if let Some(entry) = self.entries.get(&key.to_string()) {
            // Double-check mtime hasn't changed
            if let Ok(metadata) = std::fs::metadata(&chunk.file_path) {
                if metadata.modified().ok() != Some(entry.mtime) {
                    return None;  // File changed, cache invalid
                }
            }
            return Some(entry.embedding.clone());
        }
        None
    }
}
```

**Add to spec Section 5.3:** Include file mtime in cache validation for defense-in-depth.

---

### 5.3 Model Loading Race Condition

**Failure Scenario:** Two parallel processes both detect model not cached, both try to download simultaneously. Either: double download (wasted bandwidth) or one corrupts the other's download.

**Likelihood:** MEDIUM  
**Impact:** MINOR (wasteful) to MAJOR (corruption)

**Mitigation:**
```rust
/// Use lockfile for model download
fn ensure_model_downloaded(model: EmbeddingModel) -> TldrResult<PathBuf> {
    let model_dir = model_cache_dir().join(model.name());
    let lock_path = model_dir.with_extension("downloading.lock");
    
    // Check if already downloaded
    if model_dir.exists() && model_dir.join("model.onnx").exists() {
        return Ok(model_dir);
    }
    
    // Acquire exclusive lock
    let lock_file = File::create(&lock_path)?;
    lock_file.lock_exclusive()?;
    
    // Double-check after acquiring lock (another process may have completed)
    if model_dir.exists() && model_dir.join("model.onnx").exists() {
        lock_file.unlock()?;
        return Ok(model_dir);
    }
    
    // We're the first - do the download
    download_model_inner(model, &model_dir)?;
    
    lock_file.unlock()?;
    std::fs::remove_file(&lock_path).ok();  // Clean up lock file
    
    Ok(model_dir)
}
```

**Add to spec Section 3.1:** Use file lock during model download to prevent concurrent download race.

---

### 5.4 Index Build During Active Writes

**Failure Scenario:** User starts `tldr embed .` which reads many files. Meanwhile, editor auto-saves changes. Index ends up with mix of old and new embeddings, inconsistent with actual file states.

**Likelihood:** LOW  
**Impact:** MINOR (self-correcting on next run)

**Mitigation:**
```rust
/// Document limitation and provide --consistent flag
// In spec: "Index building is not atomic. If files change during indexing,
// the index may have inconsistent embeddings. Use --consistent for strict
// snapshotting (slower)."

#[arg(long)]
pub consistent: bool,  // Read all files into memory first, then embed

fn build_consistent(root: &Path, ...) -> TldrResult<Self> {
    // First pass: read all files into memory with mtimes
    let snapshots: Vec<FileSnapshot> = collect_files(root)
        .map(|p| FileSnapshot::new(p))
        .collect();
    
    // Second pass: embed from snapshots (not live files)
    // ...
}
```

**Add to spec Section 4.2:** Document non-atomic indexing. Add `--consistent` flag for strict mode.

---

## Summary of Mitigations by Priority

| Priority | Failure | Mitigation |
|----------|---------|------------|
| P0 | Cache write corruption | File locking with fs2 |
| P0 | Silent truncation | Track `truncated` flag, warn users |
| P0 | Monorepo explosion | Hard limit + helpful error |
| P1 | Opaque model download | Progress messages + --offline |
| P1 | Score confusion | Score guide + badges in output |
| P1 | Concurrent model download | Download lockfile |
| P1 | Parse failure silence | Report failures in output |
| P2 | Index staleness | Report cache age + re-index count |
| P2 | Boilerplate pollution | --no-boilerplate filter |
| P2 | ONNX version mismatch | Static linking + version check |
| P2 | Platform issues | Graceful degradation |
| P3 | Docstring dominance | DocstringMode options |
| P3 | Cross-language bias | --lang-boost option |
| P3 | Single-file edge case | Explanatory message |
| P3 | Unicode handling | Document limitation |

---

## Files to Update

1. **spec.md Section 2.1 (Types):** Add `truncated`, `original_length` to CodeChunk
2. **spec.md Section 2.4 (Index Types):** Add `language_boost`, `exclude_boilerplate` to SearchOptions
3. **spec.md Section 3.5 (Cache):** Add file locking requirement
4. **spec.md Section 4.1 (CLI):** Add `--offline`, `--exclude`, `--no-boilerplate`, `--consistent`
5. **spec.md Section 6 (Errors):** Add `IndexTooLarge`, `InsufficientDiskSpace`, `IncompatibleRuntime`
6. **spec.md Section 10 (Dependencies):** Add `fs2`, pin ONNX version, document MSRV

---

*End of Premortem Pass 2*
