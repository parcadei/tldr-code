# Premortem Analysis - Pass 3: Subtle Failure Modes

**Generated:** 2026-02-03
**Focus:** Silent failures, stale data, resource leaks, scaling failures, embedding quality

---

## 1. Silent Failures

### 1.1 Embedding Model Silently Returns Garbage on Truncation

**Scenario:** When text exceeds `max_context()` (512 tokens for most models, 8192 for ArcticMLong), the model truncates without warning. User searches for "authentication flow in the complex_auth_handler function" but the function body is 2000 tokens. Only the first 512 tokens are embedded, missing the critical auth logic at the end.

**Likelihood:** HIGH
**Impact:** CRITICAL - Users get irrelevant results and don't know why

**Mitigation:**
```rust
// In embedder.rs
pub fn embed_text(&self, text: &str) -> TldrResult<EmbeddingResult> {
    let token_count = estimate_tokens(text);
    let truncated = token_count > self.model_type.max_context();
    
    let embedding = self.model.embed(text)?;
    
    Ok(EmbeddingResult {
        embedding,
        truncated,
        original_tokens: token_count,
        embedded_tokens: token_count.min(self.model_type.max_context()),
    })
}

// In SemanticSearchReport, add:
pub truncated_chunks: usize,  // Number of chunks that were truncated
pub truncation_warning: Option<String>,  // "15 chunks exceeded 512 token limit"
```

### 1.2 Hash Collision in Cache Leading to Wrong Embeddings

**Scenario:** Cache key uses MD5/SHA256 of `file_path + content_hash + model`. If two functions have identical content but different names (copy-paste code), they share cache entries. When one is updated, the other still serves stale embedding.

**Likelihood:** MEDIUM
**Impact:** MAJOR - Similarity search returns wrong results

**Mitigation:**
```rust
// Cache key must include function identity, not just content
fn cache_key(chunk: &CodeChunk, model: EmbeddingModel) -> String {
    let mut hasher = Sha256::new();
    hasher.update(chunk.file_path.to_string_lossy().as_bytes());
    hasher.update(chunk.function_name.as_deref().unwrap_or("").as_bytes());
    hasher.update(chunk.class_name.as_deref().unwrap_or("").as_bytes());
    hasher.update(&chunk.line_start.to_le_bytes());
    hasher.update(chunk.content_hash.as_bytes());
    hasher.update(model.as_str().as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}
```

### 1.3 Parse Errors Silently Skipped Without Aggregation

**Scenario:** When chunking 1000 files, 50 have syntax errors and are silently skipped. User doesn't know 5% of their codebase is not indexed. They search for a function in a broken file and find nothing.

**Likelihood:** HIGH
**Impact:** MAJOR - Silent data loss

**Mitigation:**
```rust
// In chunk_code() return type, include skipped files
pub struct ChunkResult {
    pub chunks: Vec<CodeChunk>,
    pub skipped: Vec<SkippedFile>,
    pub stats: ChunkStats,
}

pub struct SkippedFile {
    pub path: PathBuf,
    pub reason: SkipReason,
}

pub enum SkipReason {
    ParseError(String),
    BinaryFile,
    UnsupportedLanguage(String),
    TooLarge(usize),
}

// CLI should warn: "Warning: 50 files skipped due to parse errors. Use --verbose for details."
```

### 1.4 Cosine Similarity Silently Returns NaN for Zero Vectors

**Scenario:** Empty functions or whitespace-only content produce zero vectors. `cosine_similarity(zero, any)` involves division by zero, returning NaN. This NaN propagates through `top_k_similar`, corrupting results.

**Likelihood:** MEDIUM
**Impact:** MAJOR - Entire search returns NaN scores

**Mitigation:**
```rust
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    assert_eq!(a.len(), b.len(), "Vectors must have same length");
    
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    
    // Handle zero vectors explicitly
    if norm_a < 1e-9 || norm_b < 1e-9 {
        return 0.0;  // Zero vector has 0 similarity with everything
    }
    
    (dot / (norm_a * norm_b)) as f64
}
```

---

## 2. Stale Data

### 2.1 Cache TTL Not Checked on Read

**Scenario:** Cache has `ttl_days: 30` but `get()` never checks timestamps. 6-month-old embeddings are served. Meanwhile, the embedding model was updated (ArcticM v1 -> v2), producing incompatible vectors. Old and new embeddings are compared, yielding meaningless scores.

**Likelihood:** HIGH
**Impact:** CRITICAL - Completely wrong search results

**Mitigation:**
```rust
impl EmbeddingCache {
    pub fn get(&self, chunk: &CodeChunk, model: EmbeddingModel) -> Option<Vec<f32>> {
        let key = cache_key(chunk, model);
        let entry = self.entries.get(&key)?;
        
        // Check TTL
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let age_days = (now - entry.cached_at) / 86400;
        if age_days > self.config.ttl_days as u64 {
            return None;  // Stale, force re-embed
        }
        
        // Check model version (must match exactly)
        if entry.model != model || entry.model_version != model.version() {
            return None;
        }
        
        Some(entry.embedding.clone())
    }
}

// Add version tracking to EmbeddingModel
impl EmbeddingModel {
    pub fn version(&self) -> &'static str {
        // Track fastembed model versions
        match self {
            Self::ArcticM => "5.8.0",  // Update when fastembed updates
            // ...
        }
    }
}
```

### 2.2 File Modification Not Detected Between Runs

**Scenario:** User runs `tldr embed src/`, gets cached embeddings. They modify `src/config.rs`, then run `tldr semantic "config parser"`. The search uses stale embeddings because only `content_hash` is checked, not file modification time.

**Likelihood:** HIGH (common workflow)
**Impact:** MAJOR - Search misses recent changes

**Mitigation:**
```rust
// In chunker.rs, include mtime in chunk
pub struct CodeChunk {
    // ... existing fields
    pub file_mtime: u64,  // File modification time
}

// In cache.rs, validate mtime
impl EmbeddingCache {
    pub fn get(&self, chunk: &CodeChunk, model: EmbeddingModel) -> Option<Vec<f32>> {
        let key = cache_key(chunk, model);
        let entry = self.entries.get(&key)?;
        
        // If content hash differs OR file is newer, invalidate
        if entry.content_hash != chunk.content_hash {
            return None;
        }
        if entry.file_mtime < chunk.file_mtime {
            // File was modified, content hash might be stale
            // (edge case: file touched but not changed)
            return None;
        }
        
        Some(entry.embedding.clone())
    }
}
```

### 2.3 Index Not Invalidated When Underlying Files Change

**Scenario:** User builds `SemanticIndex`, keeps it in memory, edits files, then searches. Index has stale embeddings because it was built from a snapshot.

**Likelihood:** MEDIUM (programmatic use)
**Impact:** MINOR - Mostly affects library users, not CLI

**Mitigation:**
```rust
impl SemanticIndex {
    /// Check if any indexed file has changed since index was built
    pub fn is_stale(&self) -> bool {
        for chunk in &self.chunks {
            let path = self.root.join(&chunk.chunk.file_path);
            if let Ok(meta) = fs::metadata(&path) {
                if let Ok(mtime) = meta.modified() {
                    let file_mtime = mtime.duration_since(UNIX_EPOCH).unwrap().as_secs();
                    if file_mtime > chunk.embedded_at {
                        return true;
                    }
                }
            }
        }
        false
    }
    
    /// Rebuild only changed files
    pub fn refresh(&mut self, cache: Option<&mut EmbeddingCache>) -> TldrResult<RefreshStats>;
}
```

---

## 3. Resource Leaks

### 3.1 Embedder Model Not Released After Use

**Scenario:** Each `Embedder::new()` loads a 110MB model into memory. CLI creates a new Embedder per command invocation. In long-running processes or repeated calls, memory grows unbounded.

**Likelihood:** MEDIUM
**Impact:** MAJOR - OOM in server/daemon modes

**Mitigation:**
```rust
// Use lazy_static or OnceCell for model singleton
use once_cell::sync::OnceCell;

static EMBEDDER: OnceCell<Embedder> = OnceCell::new();

impl Embedder {
    /// Get or initialize the global embedder
    pub fn global(model: EmbeddingModel) -> TldrResult<&'static Embedder> {
        EMBEDDER.get_or_try_init(|| Embedder::new(model))
    }
    
    /// Clear the global embedder (for testing or model switching)
    pub fn clear_global() {
        // Note: OnceCell doesn't support this; use parking_lot::RwLock<Option<>>
    }
}

// Alternative: Add Drop impl with explicit cleanup
impl Drop for Embedder {
    fn drop(&mut self) {
        // fastembed should handle this, but verify
        tracing::debug!("Embedder dropped, model released");
    }
}
```

### 3.2 Cache File Left Locked on Panic

**Scenario:** `EmbeddingCache::flush()` acquires file lock, starts writing, then panics mid-write (e.g., disk full). Lock is never released. Future runs hang waiting for lock.

**Likelihood:** LOW
**Impact:** MAJOR - Complete system hang

**Mitigation:**
```rust
impl EmbeddingCache {
    pub fn flush(&mut self) -> TldrResult<()> {
        if !self.dirty {
            return Ok(());
        }
        
        // Use temp file + atomic rename
        let temp_path = self.path.with_extension("tmp");
        
        // Lock scope ensures release even on panic
        {
            let file = File::create(&temp_path)?;
            let _lock = file.try_lock_exclusive()
                .map_err(|_| TldrError::CacheLocked)?;
            
            serde_json::to_writer(&file, &self.as_cache_file())?;
            file.sync_all()?;
        }  // Lock released here
        
        // Atomic rename (safe even if we panic after)
        fs::rename(&temp_path, &self.path)?;
        self.dirty = false;
        Ok(())
    }
}
```

### 3.3 Temporary Files Not Cleaned Up on Error

**Scenario:** `chunk_code()` writes intermediate results to temp files. If embedding fails mid-batch, temp files remain in `/tmp/tldr-embed-*`.

**Likelihood:** LOW
**Impact:** MINOR - Disk space leak over time

**Mitigation:**
```rust
// Use tempfile crate with auto-cleanup
use tempfile::NamedTempFile;

fn process_batch(chunks: &[CodeChunk]) -> TldrResult<Vec<EmbeddedChunk>> {
    // NamedTempFile automatically deletes on drop
    let temp = NamedTempFile::new()?;
    
    // Even if we return Err, temp is cleaned up
    // ...
}

// Add cleanup on startup
impl EmbeddingCache {
    pub fn open(config: CacheConfig) -> TldrResult<Self> {
        // Clean up orphaned temp files
        for entry in fs::read_dir(&config.cache_dir)? {
            let entry = entry?;
            if entry.path().extension() == Some("tmp".as_ref()) {
                let _ = fs::remove_file(entry.path());
            }
        }
        // ...
    }
}
```

---

## 4. Scaling Failures

### 4.1 Linear Scan Becomes Unusable at Scale

**Scenario:** At 100K functions, each search does 100K * 768 = 76.8M float operations. At 1M functions, search takes 10+ seconds.

**Likelihood:** HIGH (inevitable at scale)
**Impact:** CRITICAL - Unusable for large codebases

**Mitigation:**
```rust
// Add approximate nearest neighbor (ANN) index for large codebases
pub struct SemanticIndex {
    chunks: Vec<EmbeddedChunk>,
    
    // For small indexes: linear scan
    // For large indexes: HNSW graph
    ann_index: Option<HnswIndex>,
    
    // Threshold for switching to ANN
    const ANN_THRESHOLD: usize = 10_000,
}

impl SemanticIndex {
    pub fn search(&self, query: &str, options: SearchOptions) -> TldrResult<SemanticSearchReport> {
        if self.chunks.len() > Self::ANN_THRESHOLD {
            self.search_ann(query, options)
        } else {
            self.search_linear(query, options)
        }
    }
    
    fn search_ann(&self, query: &str, options: SearchOptions) -> TldrResult<SemanticSearchReport> {
        // Use HNSW for O(log n) search
        // Consider: usearch-rs, hora, hnswlib-rs
        todo!("Add ANN support for large codebases")
    }
}

// Spec addition: Performance guarantees
// - < 10K chunks: Linear scan, <100ms
// - 10K-100K chunks: HNSW, <200ms
// - > 100K chunks: Warn and suggest filtering, or use external vector DB
```

### 4.2 Memory Exhaustion Building Index

**Scenario:** `SemanticIndex::build()` loads all chunks into memory, then embeds all, then builds index. For 1M functions with 768-dim embeddings: 1M * 768 * 4 bytes = 3GB just for embeddings.

**Likelihood:** HIGH at scale
**Impact:** CRITICAL - OOM crash

**Mitigation:**
```rust
impl SemanticIndex {
    pub fn build(
        root: &Path,
        chunk_options: ChunkOptions,
        embed_options: EmbedOptions,
        cache: Option<&mut EmbeddingCache>,
    ) -> TldrResult<Self> {
        // Check estimated memory requirement
        let chunk_count = estimate_chunk_count(root, &chunk_options)?;
        let estimated_memory = chunk_count * embed_options.model.dimensions() * 4;
        
        if estimated_memory > MAX_INDEX_MEMORY {
            return Err(TldrError::IndexTooLarge {
                chunks: chunk_count,
                estimated_mb: estimated_memory / 1_000_000,
                suggestion: "Use --filter to reduce scope, or consider external vector DB".into(),
            });
        }
        
        // Stream chunks instead of loading all at once
        let chunks_iter = chunk_code_streaming(root, chunk_options);
        
        // Embed in batches
        let mut embedded = Vec::with_capacity(chunk_count);
        for batch in chunks_iter.chunks(embed_options.batch_size) {
            let batch_embedded = embed_batch_with_cache(batch, cache)?;
            embedded.extend(batch_embedded);
        }
        
        // ...
    }
}

const MAX_INDEX_MEMORY: usize = 500 * 1024 * 1024;  // 500MB default
```

### 4.3 Cache File Grows Unbounded

**Scenario:** Cache never evicts. After indexing 100 projects over 6 months, cache is 10GB. Parsing the JSON on startup takes 30 seconds.

**Likelihood:** HIGH
**Impact:** MAJOR - Degraded startup performance

**Mitigation:**
```rust
impl EmbeddingCache {
    pub fn open(config: CacheConfig) -> TldrResult<Self> {
        let path = config.cache_dir.join("embeddings.json");
        
        // Check cache size before loading
        if let Ok(meta) = fs::metadata(&path) {
            let size_mb = meta.len() / 1_000_000;
            if size_mb > config.max_size_mb as u64 {
                tracing::warn!(
                    "Cache exceeds max size ({} MB > {} MB), running eviction",
                    size_mb, config.max_size_mb
                );
                Self::evict_to_size(&path, config.max_size_mb)?;
            }
        }
        
        // Load with size limit
        let entries = Self::load_entries(&path, config.max_entries)?;
        
        Ok(Self { entries, dirty: false, config, path })
    }
    
    fn evict_to_size(path: &Path, max_mb: usize) -> TldrResult<()> {
        // LRU eviction: remove oldest entries until under limit
        // ...
    }
}

// Also: Use sharded cache files by project root hash
// ~/.cache/tldr/embeddings/abc123.json (per-project)
```

---

## 5. Embedding Quality Failures

### 5.1 Code Embeddings Don't Capture Semantic Intent

**Scenario:** User searches "function that validates email addresses". The codebase has `fn check_email_format(s: &str) -> bool` but the embedding model (trained on natural language) doesn't associate "validate" with "check" or "email addresses" with "email_format".

**Likelihood:** MEDIUM (model-dependent)
**Impact:** MAJOR - Poor search relevance

**Mitigation:**
```rust
// Augment code with natural language descriptions before embedding
fn prepare_text_for_embedding(chunk: &CodeChunk) -> String {
    let mut text = String::new();
    
    // Add function signature in natural language form
    if let Some(func) = &chunk.function_name {
        // Split camelCase/snake_case into words
        let words = split_identifier(func);
        text.push_str(&format!("Function: {}\n", words.join(" ")));
    }
    
    // Add docstring if available (already in spec: include_docs option)
    if let Some(doc) = extract_docstring(&chunk.content, chunk.language) {
        text.push_str(&format!("Description: {}\n", doc));
    }
    
    // Add the code
    text.push_str("Code:\n");
    text.push_str(&chunk.content);
    
    text
}

// Test embedding quality with gold-standard queries
#[cfg(test)]
mod embedding_quality_tests {
    #[test]
    fn semantic_similar_names_rank_higher() {
        // "validate email" should rank "check_email_format" above "process_data"
    }
    
    #[test]
    fn docstring_improves_ranking() {
        // Function with "validates email" in docstring should rank higher
    }
}
```

### 5.2 Different Languages Produce Incompatible Embeddings

**Scenario:** User searches across Python and Rust code. Same algorithm in both languages produces very different embeddings because embedding model hasn't seen code. Python `def foo():` vs Rust `fn foo() {}` have different surface forms.

**Likelihood:** MEDIUM
**Impact:** MINOR - Cross-language search less effective

**Mitigation:**
```rust
// Option 1: Normalize code syntax before embedding
fn normalize_code_for_embedding(content: &str, language: Language) -> String {
    // Pseudocode normalization (strip syntax-specific tokens)
    // "fn foo() -> i32 { return 42; }" -> "function foo returns integer: return 42"
    // This is complex; may not be worth it
    todo!("Consider for future enhancement")
}

// Option 2: Use code-specific embedding model
pub enum EmbeddingModel {
    // General text models
    ArcticM,
    
    // Code-specific models (future)
    CodeBERT,
    StarCoder,
    
    // Hybrid: Use code model for code, text model for queries
}

// Option 3: Document limitation and suggest language filtering
// CLI: "For best cross-language results, use --lang to filter"
```

### 5.3 Short Functions Have Poor Embeddings

**Scenario:** One-liner functions like `fn is_empty(&self) -> bool { self.len() == 0 }` produce poor embeddings because there's not enough context. Search for "check if empty" misses these.

**Likelihood:** HIGH
**Impact:** MINOR - One-liners often found by name search anyway

**Mitigation:**
```rust
// Augment short functions with context
fn prepare_text_for_embedding(chunk: &CodeChunk) -> String {
    let content_len = chunk.content.len();
    
    if content_len < 100 {  // Short function
        // Add class context if available
        let context = if let Some(class) = &chunk.class_name {
            format!("Method of class {}\n", class)
        } else {
            String::new()
        };
        
        // Add expanded description
        let expanded = expand_short_function(&chunk.content, chunk.language);
        
        format!("{}{}\nCode: {}", context, expanded, chunk.content)
    } else {
        chunk.content.clone()
    }
}

fn expand_short_function(content: &str, lang: Language) -> String {
    // "fn is_empty(&self) -> bool { self.len() == 0 }"
    // -> "Checks if empty by comparing length to zero. Returns boolean."
    // Use simple heuristics, not LLM
    todo!("Implement short function expansion")
}
```

### 5.4 Query-Document Mismatch

**Scenario:** Model is symmetric (same embedding for query and document). But optimal retrieval uses asymmetric models where queries and documents have different embedding spaces. User queries like "how to authenticate" don't match document embeddings of actual auth code.

**Likelihood:** MEDIUM
**Impact:** MINOR - Arctic models handle this reasonably well

**Mitigation:**
```rust
// Use query-specific prefix as recommended by Snowflake Arctic docs
impl Embedder {
    pub fn embed_query(&self, query: &str) -> TldrResult<Vec<f32>> {
        // Arctic models use different prefixes for queries vs documents
        let prefixed = format!("query: {}", query);
        self.embed_text(&prefixed)
    }
    
    pub fn embed_document(&self, doc: &str) -> TldrResult<Vec<f32>> {
        // Documents get different prefix
        let prefixed = format!("passage: {}", doc);
        self.embed_text(&prefixed)
    }
}

// Update all call sites:
// - Index building: embed_document()
// - Search: embed_query()
```

---

## Summary: Critical Mitigations to Add to Spec

### Must Have (P0)

1. **Truncation warning** - Tell users when chunks exceed token limit
2. **Cache TTL enforcement** - Check timestamps on every read
3. **Zero vector handling** - Return 0.0 similarity, don't NaN
4. **Skipped files reporting** - Surface parse errors to user
5. **Memory limit check** - Fail fast before OOM

### Should Have (P1)

6. **File mtime validation** - Detect changes between runs
7. **Atomic cache writes** - Prevent corruption on crash
8. **Index staleness check** - Warn when files changed since build
9. **Query/document prefixes** - Use proper Arctic embedding format
10. **Cache size limit** - LRU eviction when over limit

### Nice to Have (P2)

11. **ANN index** - HNSW for >10K chunks
12. **Short function augmentation** - Better embeddings for one-liners
13. **Model singleton** - Avoid repeated model loading
14. **Per-project cache sharding** - Faster cache operations

---

## Spec Additions Required

Add to Section 6 (Error Handling):
```markdown
### 6.4 Graceful Handling of Edge Cases

| Edge Case | Behavior |
|-----------|----------|
| Text exceeds max_context | Truncate + set `truncated=true` in result |
| Zero/empty embedding | Return zero vector, similarity=0.0 |
| Cache entry expired | Cache miss, re-embed |
| File modified since index | Warning, suggest rebuild |
| Index exceeds memory limit | Error with suggestion to filter |
```

Add to Section 8 (Performance):
```markdown
### 8.5 Scaling Limits

| Scale | Search Method | Expected Latency |
|-------|---------------|------------------|
| < 10K chunks | Linear scan | < 100ms |
| 10K-100K chunks | Linear scan + warning | < 1s |
| > 100K chunks | Error, suggest filtering | N/A |

Future: Add HNSW index for O(log n) search at scale.
```

Add to Section 5 (Behavioral Contracts):
```markdown
### 5.5 Embedding Contracts

| Input | Output | Metadata |
|-------|--------|----------|
| Text > max_context | Truncated embedding | `truncated: true` |
| Empty text | Zero vector | `is_zero: true` |
| Query text | Embedding with "query:" prefix | - |
| Document text | Embedding with "passage:" prefix | - |
```

