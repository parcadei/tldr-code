# Premortem Analysis: Semantic Search Module - Pass 1/3
**Date:** 2026-02-03  
**Status:** Pre-implementation failure analysis  
**Assumption:** The project has FAILED. Why?

---

## Executive Summary

This premortem assumes the semantic search module implementation has failed. Below are the identified failure modes, their likelihood and impact, and concrete mitigations that should be added to the specification before implementation begins.

---

## 1. Dependency Failures

### 1.1 fastembed-rs ONNX Runtime Breaks

**Failure Scenario:** fastembed-rs v5.8 depends on ort (ONNX Runtime) which has complex native library dependencies. Different platforms (macOS ARM, macOS x86, Linux x86, Windows) require different prebuilt libraries. A version mismatch or missing library causes `ModelLoadError` at runtime.

**Likelihood:** HIGH  
**Impact:** CRITICAL (entire feature is blocked)

**Evidence:**
- ort crate has history of platform-specific issues (especially Apple Silicon vs Intel)
- fastembed downloads runtime at build time which can fail in CI or air-gapped environments
- ONNX Runtime version must match model format version

**Mitigation to add to spec:**

```rust
// Section 6.2 Error Scenarios - ADD:
/// ONNX runtime compatibility check
/// 
/// Before attempting to load any model, verify ONNX runtime is functional:
/// 1. Check for ORT_DYLIB_PATH environment variable (custom library location)
/// 2. Validate runtime version matches expected version (1.16+)
/// 3. Test with a minimal operation before loading large model
/// 
/// If check fails:
/// - Log detailed error with platform info (arch, OS version)
/// - Suggest: `export ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib`
/// - Provide link to troubleshooting docs
pub fn validate_onnx_runtime() -> Result<(), OnnxRuntimeError>;

// Add to Section 6.3 Graceful Degradation:
/// If ONNX runtime fails to initialize:
/// 1. Emit warning: "Semantic search unavailable: ONNX runtime not found"
/// 2. Return `TldrError::SemanticUnavailable` (new error type)
/// 3. CLI falls back to BM25 search with message
/// 4. Do NOT panic or exit - other tldr commands should still work
```

**Test to add:**
```rust
#[test]
fn onnx_runtime_unavailable_graceful_fallback() {
    // Simulate missing ONNX runtime
    // Verify that `tldr semantic` returns BM25 results with warning
    // NOT a crash
}
```

---

### 1.2 fastembed API Breaking Changes

**Failure Scenario:** fastembed-rs is at v5.8 and actively developed. A minor version bump (5.9) changes the TextEmbedding API, breaking compilation.

**Likelihood:** MEDIUM  
**Impact:** MAJOR (requires code changes to fix)

**Evidence:**
- fastembed-rs v4 to v5 had significant API changes
- Enum variants for models change between versions
- No stable API guarantee

**Mitigation to add to spec:**

```toml
# Section 10.1 - CHANGE Cargo.toml dependency:
[dependencies]
# Pin to exact version to prevent breaking changes
# Review and update manually after testing
fastembed = "=5.8.1"  # PINNED - do not use ^5.8 or 5
```

```rust
// Add version check at compile time
#[cfg(not(feature = "fastembed_5_8"))]
compile_error!("This module requires fastembed 5.8.x. Update carefully.");
```

**Action:** Add a `DEPENDENCY_VERSIONS.md` file documenting tested versions.

---

### 1.3 Model Download Fails

**Failure Scenario:** First-time users run `tldr semantic "query"` and the 110MB model download fails due to:
- Network timeout
- Firewall blocking HuggingFace
- Disk quota exceeded in `~/.cache/fastembed/`
- HuggingFace CDN outage

**Likelihood:** HIGH  
**Impact:** MAJOR (first-time experience broken)

**Evidence:**
- No offline bundling mentioned in spec
- Default behavior is automatic download
- Enterprise users often have network restrictions

**Mitigation to add to spec:**

```rust
// Section 3.1 Embedder - ADD initialization flow:
impl Embedder {
    /// Create embedder with download options
    pub fn new_with_options(model: EmbeddingModel, options: ModelOptions) -> TldrResult<Self>;
}

pub struct ModelOptions {
    /// Directory to cache models (default: ~/.cache/fastembed/)
    pub cache_dir: Option<PathBuf>,
    
    /// Timeout for model download in seconds (default: 300)
    pub download_timeout_secs: u64,
    
    /// Skip download if model not present (return error instead)
    pub offline_mode: bool,
    
    /// Custom HuggingFace endpoint (for mirrors/proxies)
    pub hf_endpoint: Option<String>,
}

// Section 4.1 CLI - ADD flags:
#[arg(long)]
/// Fail if model needs downloading (offline mode)
pub offline: bool,

#[arg(long, env = "TLDR_MODEL_CACHE")]
/// Custom directory for model cache
pub model_cache: Option<PathBuf>,
```

**Add CLI command:**
```bash
# Pre-download model explicitly
tldr model download arctic-m
tldr model list  # Show downloaded models
tldr model path arctic-m  # Print cache path
```

---

## 2. Performance Failures

### 2.1 Embedding Too Slow for Real Codebases

**Failure Scenario:** User runs `tldr embed src/` on a real codebase (5000+ functions). Expected time: ~5 seconds. Actual time: 5+ minutes. Users abandon the tool.

**Likelihood:** HIGH  
**Impact:** MAJOR (unusable for real projects)

**Evidence:**
- Spec says "~50ms for 100 texts" but real codebases have 5000+ functions
- 5000 functions = 50 batches of 100 = 2500ms embedding time alone
- Plus parsing + I/O + cache writes
- First run (no cache) is particularly slow

**Mitigation to add to spec:**

```rust
// Section 8.3 Optimization Strategies - EXPAND:

/// Parallel chunking with rayon
/// Files are parsed in parallel using rayon's thread pool
/// Default parallelism: num_cpus
use rayon::prelude::*;

pub fn chunk_code_parallel(path: &Path, options: ChunkOptions) -> TldrResult<Vec<CodeChunk>> {
    let files = collect_source_files(path)?;
    files.par_iter()
        .filter_map(|f| chunk_file(f, &options).ok())
        .flatten()
        .collect()
}

/// Streaming embedding with progress callback
pub fn embed_with_progress<F>(
    chunks: &[CodeChunk],
    options: EmbedOptions,
    progress: F,
) -> TldrResult<Vec<EmbeddedChunk>>
where
    F: Fn(usize, usize),  // (completed, total)
```

**Add performance targets to spec Section 8:**
```markdown
### 8.5 Performance Targets (SLOs)

| Codebase Size | First Run (no cache) | Subsequent (cached) |
|---------------|---------------------|---------------------|
| 100 functions | < 3 seconds | < 500ms |
| 1000 functions | < 15 seconds | < 1 second |
| 5000 functions | < 60 seconds | < 3 seconds |
| 10000 functions | < 120 seconds | < 5 seconds |

If embedding exceeds 60 seconds, show:
- Progress bar with ETA
- Option to abort (Ctrl+C gracefully)
- Suggestion: "Use --model xs for faster (lower quality) embeddings"
```

---

### 2.2 Search Latency Unacceptable at Scale

**Failure Scenario:** With 10K functions, linear search O(n*d) takes too long. User queries take 500ms+ instead of expected <100ms.

**Likelihood:** MEDIUM  
**Impact:** MAJOR (interactive use broken)

**Evidence:**
- Linear scan: 10K chunks * 768 dims = 7.68M multiply-adds per query
- Spec mentions "~10ms for 10K chunks" but this assumes SIMD optimization
- Rust default f32 operations may not be vectorized

**Mitigation to add to spec:**

```rust
// Section 3.4 Similarity - ADD SIMD optimization:

/// Use SIMD for cosine similarity when available
/// 
/// On x86_64: Uses AVX2/AVX-512 via std::simd or simdeez
/// On aarch64: Uses NEON
/// Fallback: Scalar implementation
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// Benchmark: 10K vectors @ 768 dims
/// - Scalar: ~15ms
/// - AVX2: ~3ms
/// - AVX-512: ~1.5ms
pub fn cosine_similarity_simd(a: &[f32], b: &[f32]) -> f64;

// Section 8.4 Memory Limits - ADD:
/// For indices > 50K chunks, suggest ANN indexing
pub const ANN_THRESHOLD: usize = 50_000;

impl SemanticIndex {
    pub fn build(...) -> TldrResult<Self> {
        let chunks = /* ... */;
        if chunks.len() > ANN_THRESHOLD {
            eprintln!("Warning: {} chunks exceeds linear search threshold.", chunks.len());
            eprintln!("Consider: tldr semantic --ann-index for large codebases");
        }
        // ...
    }
}
```

**Add to Cargo.toml:**
```toml
[dependencies]
# SIMD acceleration for similarity search
simdeez = { version = "1.0", optional = true }

[features]
simd = ["simdeez"]
```

---

## 3. Memory Failures

### 3.1 10K Functions Exhausts Memory

**Failure Scenario:** User with 10K functions loads all embeddings into memory. 768 dims * 4 bytes * 10K = 30MB for vectors alone. But CodeChunk also stores content strings. A codebase with average 50 lines per function * 80 chars * 10K = 40MB just for content. Plus metadata, hash maps, etc. Total: 100MB+ which causes OOM on constrained systems.

**Likelihood:** MEDIUM  
**Impact:** MAJOR (crash on large codebases)

**Evidence:**
- Spec estimates 30MB for vectors but ignores content storage
- Docker containers often have 256MB-512MB limits
- CI runners may be memory-constrained

**Mitigation to add to spec:**

```rust
// Section 2.1 Core Types - MODIFY CodeChunk:

/// A chunk of code that can be embedded
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChunk {
    // ... existing fields ...
    
    /// The source code text
    /// NOTE: For memory efficiency, consider storing only first 500 chars
    /// Full content can be read from file when needed
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content: String,
    
    /// Content truncated flag
    #[serde(default)]
    pub content_truncated: bool,
}

// Section 8.4 Memory Limits - EXPAND:

/// Maximum content size stored per chunk (bytes)
/// Full content read from file on demand
pub const MAX_STORED_CONTENT: usize = 512;

/// Estimate memory usage for index
pub fn estimate_memory_bytes(chunk_count: usize, dims: usize) -> usize {
    let vectors = chunk_count * dims * 4;  // f32 = 4 bytes
    let metadata = chunk_count * 200;      // PathBuf, strings, etc.
    let content = chunk_count * MAX_STORED_CONTENT;
    vectors + metadata + content
}

impl SemanticIndex {
    /// Check if index would exceed memory limit
    pub fn would_exceed_memory(&self, max_mb: usize) -> bool {
        let estimated = estimate_memory_bytes(self.chunks.len(), self.model.dimensions());
        estimated > max_mb * 1024 * 1024
    }
}
```

**Add CLI option:**
```rust
#[arg(long, default_value = "500")]
/// Maximum memory usage in MB (will warn if exceeded)
pub max_memory_mb: usize,
```

---

### 3.2 Cache File Grows Unbounded

**Failure Scenario:** User works on multiple projects over months. Each project adds embeddings to global cache. Cache grows to multiple GB, filling disk.

**Likelihood:** MEDIUM  
**Impact:** MINOR (disk full, but recoverable)

**Evidence:**
- Cache is global (`~/.cache/tldr/embeddings/`)
- No automatic eviction in spec
- Each embedding is ~3KB, 100K embeddings = 300MB
- Plus old versions accumulate

**Mitigation to add to spec:**

```rust
// Section 3.5 Cache - EXPAND eviction policy:

impl EmbeddingCache {
    /// Evict entries to stay under size limit
    /// 
    /// Strategy: LRU eviction based on last_accessed timestamp
    /// 
    /// Called automatically when:
    /// - Adding new entry would exceed max_size_mb
    /// - On startup if cache exceeds 2x max_size_mb
    pub fn enforce_size_limit(&mut self) -> usize {
        if self.size_bytes() <= self.config.max_size_mb * 1024 * 1024 {
            return 0;
        }
        
        // Sort by last_accessed ascending (oldest first)
        // Evict until under limit
        let mut evicted = 0;
        while self.size_bytes() > self.config.max_size_mb * 1024 * 1024 {
            if let Some(oldest) = self.find_oldest_entry() {
                self.remove(&oldest);
                evicted += 1;
            } else {
                break;
            }
        }
        evicted
    }
}

// Update CacheEntry:
struct CacheEntry {
    // ... existing fields ...
    
    /// Last time this entry was accessed (for LRU eviction)
    last_accessed: u64,
}

// CLI command:
/// Clean up embedding cache
/// $ tldr cache clean --older-than 30d --dry-run
/// $ tldr cache stats
```

---

## 4. Model Failures

### 4.1 Model Download Corrupted

**Failure Scenario:** Model download is interrupted or corrupted. fastembed loads garbage, produces nonsense embeddings. Search results are random. User doesn't realize embeddings are wrong.

**Likelihood:** LOW  
**Impact:** CRITICAL (silent wrong results)

**Evidence:**
- Network interruptions common
- No checksum verification mentioned
- ONNX models can partially load with corrupted weights

**Mitigation to add to spec:**

```rust
// Section 3.1 Embedder - ADD validation:

impl Embedder {
    pub fn new(model: EmbeddingModel) -> TldrResult<Self> {
        let embedder = Self::load_model(model)?;
        
        // Validate model produces expected output
        embedder.validate_model_integrity()?;
        
        Ok(embedder)
    }
    
    /// Validate model by checking known input/output pair
    fn validate_model_integrity(&self) -> TldrResult<()> {
        const TEST_INPUT: &str = "def hello(): pass";
        const EXPECTED_NORM: f64 = 1.0;  // Normalized
        
        let embedding = self.embed_text(TEST_INPUT)?;
        
        // Check dimensions
        if embedding.len() != self.model.dimensions() {
            return Err(TldrError::ModelCorrupted {
                expected_dims: self.model.dimensions(),
                actual_dims: embedding.len(),
            });
        }
        
        // Check normalization (sanity check)
        let norm: f64 = embedding.iter()
            .map(|x| (*x as f64) * (*x as f64))
            .sum::<f64>()
            .sqrt();
        if (norm - EXPECTED_NORM).abs() > 0.1 {
            return Err(TldrError::ModelCorrupted {
                detail: format!("Embedding norm {} far from expected {}", norm, EXPECTED_NORM),
            });
        }
        
        Ok(())
    }
}

// Add error type:
#[error("Embedding model corrupted: {detail}. Try: rm -rf ~/.cache/fastembed/ && tldr model download")]
ModelCorrupted { detail: String },
```

---

### 4.2 Model Produces Inconsistent Embeddings

**Failure Scenario:** Same code chunk produces different embeddings on different runs. Cache invalidation fails, search results are unstable.

**Likelihood:** LOW  
**Impact:** MAJOR (cache useless, unstable results)

**Evidence:**
- ONNX models are generally deterministic, but floating point precision varies
- Different ONNX Runtime versions may produce slightly different results
- GPU vs CPU execution can differ

**Mitigation to add to spec:**

```rust
// Section 5.1 Embedding Invariants - ADD determinism test:

#[test]
fn embedding_determinism() {
    let embedder = Embedder::new(EmbeddingModel::ArcticM).unwrap();
    let text = "fn process_data(input: &str) -> Result<Output>";
    
    let e1 = embedder.embed_text(text).unwrap();
    let e2 = embedder.embed_text(text).unwrap();
    
    // Must be EXACTLY equal (not just approximately)
    assert_eq!(e1, e2, "Embeddings must be deterministic");
}

// Add to spec Section 5.1:
/// Determinism requirement: Same text + model MUST produce bit-identical embeddings
/// This is required for cache correctness.
/// If ONNX runtime cannot guarantee this, add epsilon tolerance to cache keys.
```

---

## 5. Integration Failures

### 5.1 Tree-sitter Extraction Misses Functions

**Failure Scenario:** tree-sitter parser doesn't extract all functions:
- Lambda functions missed
- Nested functions missed
- Methods in unusual syntax missed (e.g., decorators confuse parser)
- Async functions handled differently

User searches for code that exists but gets no results because it wasn't chunked.

**Likelihood:** HIGH  
**Impact:** MAJOR (incomplete search results)

**Evidence:**
- Python lambdas: `lambda x: x + 1` - not a "function definition"
- JavaScript arrow functions: `const f = (x) => x`
- Rust closures: `|x| x + 1`
- Class methods with decorators may confuse extraction

**Mitigation to add to spec:**

```rust
// Section 3.2 Chunker - ADD comprehensive extraction:

/// Function-like constructs to extract per language
/// 
/// Python:
/// - function_definition (def)
/// - lambda (inline)
/// - decorated_function
/// - method_definition (in class)
/// - async_function_definition
/// 
/// Rust:
/// - function_item (fn)
/// - closure_expression (|x| ...)
/// - impl method
/// - async fn
/// 
/// TypeScript/JavaScript:
/// - function_declaration
/// - arrow_function
/// - method_definition
/// - generator_function
/// - async_function
const FUNCTION_NODE_TYPES: &[(&str, &[&str])] = &[
    ("python", &["function_definition", "lambda", "async_function_definition"]),
    ("rust", &["function_item", "closure_expression"]),
    ("typescript", &["function_declaration", "arrow_function", "method_definition"]),
    // ...
];

/// Extract ALL function-like constructs, not just top-level functions
fn extract_all_functions(tree: &Tree, source: &str, lang: Language) -> Vec<CodeChunk> {
    // Use tree-sitter query to find ALL function types
    // Including nested functions and closures
}
```

**Add test:**
```rust
#[test]
fn chunk_file_extracts_lambdas_and_closures() {
    // Python with lambda
    let py = "f = lambda x: x + 1\ndef g(): pass";
    // Should extract both lambda and g
    
    // Rust with closure
    let rs = "fn main() { let f = |x| x + 1; }";
    // Should extract main AND the closure
}
```

---

### 5.2 AST Module API Mismatch

**Failure Scenario:** The semantic chunker depends on `tldr_core::ast::extractor::extract_functions` but that function's signature or return type changes. Compilation fails.

**Likelihood:** MEDIUM  
**Impact:** MAJOR (build broken)

**Evidence:**
- AST module is in active development
- No versioned internal API
- Spec assumes certain function signatures exist

**Mitigation to add to spec:**

```rust
// Section 7.1 - ADD adapter layer:

/// Adapter for AST module to isolate semantic module from internal changes
/// 
/// This layer translates between AST module types and semantic module types,
/// providing a stable interface even if AST internals change.
mod ast_adapter {
    use crate::ast;
    use super::types::CodeChunk;
    
    /// Extract functions from a file, converting to CodeChunks
    /// 
    /// If AST module API changes, update THIS function only.
    pub fn extract_functions_as_chunks(
        path: &Path,
    ) -> TldrResult<Vec<CodeChunk>> {
        let (tree, source, _) = ast::parser::parse_file(path)?;
        let language = Language::from_path(path)?;
        
        // Use AST extractor, convert results
        let functions = ast::extractor::extract_functions(&tree, &source, language);
        
        functions.into_iter()
            .map(|f| convert_to_chunk(f, path, &source, language))
            .collect()
    }
    
    fn convert_to_chunk(
        func: ast::Function,  // AST module type
        path: &Path,
        source: &str,
        language: Language,
    ) -> TldrResult<CodeChunk> {
        // Conversion logic - single point of adaptation
    }
}
```

---

### 5.3 Language Detection Fails

**Failure Scenario:** File has wrong extension or no extension. Language::from_path returns None. File is skipped silently. User's code isn't indexed.

**Likelihood:** MEDIUM  
**Impact:** MINOR (some files missed)

**Evidence:**
- Shell scripts often have no extension
- Config files might contain code snippets
- Polyglot files (e.g., Jupyter notebooks)

**Mitigation to add to spec:**

```rust
// Section 3.2 Chunker - ADD language detection fallback:

/// Detect language with fallback strategies
/// 
/// 1. File extension (fastest)
/// 2. Shebang line (#!/usr/bin/env python)
/// 3. Content heuristics (import statements, syntax patterns)
fn detect_language(path: &Path, content: &str) -> Option<Language> {
    // Try extension first
    if let Some(lang) = Language::from_path(path) {
        return Some(lang);
    }
    
    // Try shebang
    if let Some(first_line) = content.lines().next() {
        if first_line.starts_with("#!") {
            if first_line.contains("python") { return Some(Language::Python); }
            if first_line.contains("node") { return Some(Language::TypeScript); }
            if first_line.contains("ruby") { return Some(Language::Ruby); }
            // ...
        }
    }
    
    // Try content heuristics
    if content.contains("def ") && content.contains(":") {
        return Some(Language::Python);
    }
    if content.contains("fn ") && content.contains("->") {
        return Some(Language::Rust);
    }
    
    None
}

// Log skipped files:
if language.is_none() {
    tracing::debug!("Skipping {}: unable to detect language", path.display());
}
```

---

## 6. Additional Failure Modes

### 6.1 Unicode/Encoding Issues

**Failure Scenario:** Source file uses non-UTF-8 encoding (Windows-1252, Latin-1). Content parsing fails or produces garbage. Embeddings are meaningless.

**Likelihood:** LOW  
**Impact:** MINOR (some files corrupted)

**Mitigation:**
```rust
// Validate UTF-8 and skip non-UTF-8 files with warning
let content = match std::fs::read_to_string(path) {
    Ok(c) => c,
    Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
        tracing::warn!("Skipping {}: not valid UTF-8", path.display());
        return Ok(vec![]);
    }
    Err(e) => return Err(e.into()),
};
```

### 6.2 Concurrent Access to Cache

**Failure Scenario:** Two `tldr semantic` processes run simultaneously. Both try to write to cache file. File corruption or lost writes.

**Likelihood:** LOW  
**Impact:** MINOR (cache needs rebuild)

**Mitigation:**
```rust
// Section 5.3 Caching Invariants - already mentions locking
// ADD implementation detail:
use fs2::FileExt;  // File locking

impl EmbeddingCache {
    fn lock_for_write(&self) -> TldrResult<std::fs::File> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(&self.lock_path)?;
        file.lock_exclusive()?;
        Ok(file)
    }
}
```

### 6.3 Test Fixtures Missing

**Failure Scenario:** Tests depend on `fixtures/simple-project/` but fixture files are missing or wrong. Tests fail in CI.

**Likelihood:** HIGH (during initial development)  
**Impact:** MINOR (tests fail, not production)

**Mitigation:**
```rust
// In tests - create fixtures programmatically:
fn create_test_project(dir: &Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        "pub fn process_data() { }\npub fn validate() { }"
    ).unwrap();
    std::fs::write(
        dir.join("main.py"),
        "def main():\n    pass\n\ndef process_data(x):\n    return x"
    ).unwrap();
}
```

---

## Summary: Required Spec Amendments

| Section | Amendment | Priority |
|---------|-----------|----------|
| 6.2 Error Scenarios | Add ONNX runtime validation | HIGH |
| 6.3 Graceful Degradation | Fallback when semantic unavailable | HIGH |
| 10.1 Dependencies | Pin fastembed to exact version | HIGH |
| 3.1 Embedder | Add ModelOptions with offline mode | HIGH |
| 8.3 Optimization | Add parallel chunking with rayon | HIGH |
| 8.5 Performance Targets | Add SLOs for different codebase sizes | HIGH |
| 3.4 Similarity | Add SIMD optimization option | MEDIUM |
| 8.4 Memory Limits | Add content truncation, memory estimation | MEDIUM |
| 3.5 Cache | Add LRU eviction with size limits | MEDIUM |
| 3.1 Embedder | Add model integrity validation | MEDIUM |
| 3.2 Chunker | Comprehensive function extraction (lambdas, closures) | HIGH |
| 7.1 AST Integration | Add adapter layer for AST module | MEDIUM |
| 3.2 Chunker | Multi-strategy language detection | LOW |
| 3.5 Cache | File locking for concurrent access | LOW |

---

## Next Steps

1. Review this analysis with the team
2. Incorporate HIGH priority mitigations into spec v1.1
3. Create tickets for MEDIUM priority items
4. Proceed with implementation using amended spec

---

*End of Premortem Pass 1*
