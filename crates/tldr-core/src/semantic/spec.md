# Semantic Search Module Specification

**Version:** 1.0  
**Created:** 2026-02-03  
**Author:** architect-agent  
**Status:** Approved for Implementation

## Overview

This specification defines the semantic code search module for tldr-rs, providing AI-powered code search using dense embeddings. The module enables natural language queries to find semantically related code, similarity detection between code fragments, and embedding generation for downstream tools.

## Table of Contents

1. [Module Structure](#1-module-structure)
2. [Public API Types](#2-public-api-types)
3. [Core Components](#3-core-components)
4. [CLI Commands](#4-cli-commands)
5. [Behavioral Contracts](#5-behavioral-contracts)
6. [Error Handling](#6-error-handling)
7. [Integration Points](#7-integration-points)
8. [Performance Considerations](#8-performance-considerations)
9. [Testing Strategy](#9-testing-strategy)

---

## 1. Module Structure

```
tldr-core/src/semantic/
├── mod.rs              # Module exports and re-exports
├── spec.md             # This specification
├── embedder.rs         # Embedding generation (fastembed-rs wrapper)
├── chunker.rs          # Code chunking via tree-sitter
├── index.rs            # In-memory embedding index
├── cache.rs            # JSON-based embedding cache
├── similarity.rs       # Cosine similarity and search
└── types.rs            # Shared types (CodeChunk, EmbeddingResult, etc.)

tldr-cli/src/commands/
├── semantic.rs         # `tldr semantic <query> [path]`
├── embed.rs            # `tldr embed <file|path>`
├── similar.rs          # `tldr similar <file>`
└── explain.rs          # `tldr explain <file> <function>` (DEFERRED)
```

### Module Dependencies

```
┌─────────────────────────────────────────────────────────────────┐
│                         tldr-cli                                │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐           │
│  │ semantic │ │  embed   │ │ similar  │ │ explain  │           │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘           │
│       │            │            │            │ (DEFERRED)       │
└───────┼────────────┼────────────┼────────────┼──────────────────┘
        │            │            │            │
        ▼            ▼            ▼            ▼
┌─────────────────────────────────────────────────────────────────┐
│                       tldr-core/semantic                        │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                       index.rs                            │  │
│  │  SemanticIndex::search() / SemanticIndex::find_similar() │  │
│  └────────────────────────┬─────────────────────────────────┘  │
│                           │                                     │
│  ┌────────────┐  ┌────────┴───────┐  ┌────────────┐            │
│  │ embedder.rs│◄─┤  similarity.rs │──►  cache.rs  │            │
│  └─────┬──────┘  └────────────────┘  └─────┬──────┘            │
│        │                                    │                   │
│  ┌─────▼──────┐                      ┌─────▼──────┐            │
│  │ chunker.rs │                      │  types.rs  │            │
│  └─────┬──────┘                      └────────────┘            │
│        │                                                        │
└────────┼────────────────────────────────────────────────────────┘
         │
         ▼
┌─────────────────────┐     ┌─────────────────────┐
│ tldr-core/ast       │     │    fastembed-rs     │
│ (tree-sitter)       │     │ (embedding models)  │
└─────────────────────┘     └─────────────────────┘
```

---

## 2. Public API Types

### 2.1 Core Types (`types.rs`)

```rust
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

/// A chunk of code that can be embedded
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChunk {
    /// Source file path (relative to project root)
    pub file_path: PathBuf,
    
    /// Function/method name (None for file-level chunks)
    pub function_name: Option<String>,
    
    /// Class/struct name containing this function (if any)
    pub class_name: Option<String>,
    
    /// Start line number (1-indexed)
    pub line_start: u32,
    
    /// End line number (1-indexed, inclusive)
    pub line_end: u32,
    
    /// The source code text
    pub content: String,
    
    /// Content hash for cache invalidation (MD5 or SHA-256)
    pub content_hash: String,
    
    /// Language of the code
    pub language: Language,
}

/// Embedding result for a code chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedChunk {
    /// The original code chunk
    pub chunk: CodeChunk,
    
    /// Dense embedding vector (768 dimensions for Arctic-M)
    pub embedding: Vec<f32>,
    
    /// Model used to generate embedding
    pub model: EmbeddingModel,
    
    /// Timestamp when embedding was generated (Unix epoch seconds)
    pub embedded_at: u64,
}

/// Supported embedding models (Snowflake Arctic family)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingModel {
    /// 384 dims, 30MB, 512 context
    ArcticXS,
    /// 384 dims, 90MB, 512 context  
    ArcticS,
    /// 768 dims, 110MB, 512 context (DEFAULT)
    #[default]
    ArcticM,
    /// 768 dims, 110MB, 8192 context
    ArcticMLong,
    /// 1024 dims, 335MB, 512 context
    ArcticL,
}

impl EmbeddingModel {
    /// Get embedding dimension for this model
    pub fn dimensions(&self) -> usize {
        match self {
            Self::ArcticXS | Self::ArcticS => 384,
            Self::ArcticM | Self::ArcticMLong => 768,
            Self::ArcticL => 1024,
        }
    }
    
    /// Get max context length (tokens)
    pub fn max_context(&self) -> usize {
        match self {
            Self::ArcticMLong => 8192,
            _ => 512,
        }
    }
    
    /// Get fastembed model enum variant
    pub fn fastembed_model(&self) -> fastembed::EmbeddingModel {
        match self {
            Self::ArcticXS => fastembed::EmbeddingModel::SnowflakeArcticEmbedXS,
            Self::ArcticS => fastembed::EmbeddingModel::SnowflakeArcticEmbedS,
            Self::ArcticM => fastembed::EmbeddingModel::SnowflakeArcticEmbedM,
            Self::ArcticMLong => fastembed::EmbeddingModel::SnowflakeArcticEmbedMLong,
            Self::ArcticL => fastembed::EmbeddingModel::SnowflakeArcticEmbedL,
        }
    }
}

/// Granularity for code chunking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ChunkGranularity {
    /// One chunk per file
    File,
    /// One chunk per function/method (DEFAULT)
    #[default]
    Function,
}

/// Semantic search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSearchResult {
    /// File path
    pub file_path: PathBuf,
    
    /// Function name (if function-level)
    pub function_name: Option<String>,
    
    /// Class name (if method)
    pub class_name: Option<String>,
    
    /// Cosine similarity score (0.0 to 1.0)
    pub score: f64,
    
    /// Start line
    pub line_start: u32,
    
    /// End line
    pub line_end: u32,
    
    /// Code snippet (truncated for display)
    pub snippet: String,
}

/// Report from semantic search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSearchReport {
    /// Search results sorted by score (descending)
    pub results: Vec<SemanticSearchResult>,
    
    /// Original query
    pub query: String,
    
    /// Model used for query embedding
    pub model: EmbeddingModel,
    
    /// Total chunks searched
    pub total_chunks: usize,
    
    /// Results above threshold
    pub matches_above_threshold: usize,
    
    /// Search latency in milliseconds
    pub latency_ms: u64,
    
    /// Whether cache was used
    pub cache_hit: bool,
}

/// Report from embedding generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedReport {
    /// Path that was embedded
    pub path: PathBuf,
    
    /// Model used
    pub model: EmbeddingModel,
    
    /// Granularity used
    pub granularity: ChunkGranularity,
    
    /// Number of chunks embedded
    pub chunks_embedded: usize,
    
    /// Number of chunks loaded from cache
    pub chunks_cached: usize,
    
    /// Embedded chunks (if output requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks: Option<Vec<EmbeddedChunk>>,
    
    /// Total embedding time in milliseconds
    pub latency_ms: u64,
}

/// Report from similarity search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarityReport {
    /// Source file/function being compared
    pub source: CodeChunk,
    
    /// Similar code fragments
    pub similar: Vec<SemanticSearchResult>,
    
    /// Model used
    pub model: EmbeddingModel,
    
    /// Total chunks compared
    pub total_compared: usize,
    
    /// Whether self was excluded
    pub exclude_self: bool,
}
```

### 2.2 Embedder Types (`embedder.rs`)

```rust
use fastembed::TextEmbedding;

/// Embedding service using fastembed-rs
pub struct Embedder {
    /// The fastembed TextEmbedding instance
    model: TextEmbedding,
    
    /// Model variant being used
    model_type: EmbeddingModel,
    
    /// Whether model is loaded
    initialized: bool,
}

/// Options for embedding generation
#[derive(Debug, Clone, Default)]
pub struct EmbedOptions {
    /// Model to use (default: ArcticM)
    pub model: EmbeddingModel,
    
    /// Show progress during embedding
    pub show_progress: bool,
    
    /// Batch size for embedding (default: 32)
    pub batch_size: usize,
}
```

### 2.3 Chunker Types (`chunker.rs`)

```rust
/// Code chunking options
#[derive(Debug, Clone, Default)]
pub struct ChunkOptions {
    /// Granularity (file or function)
    pub granularity: ChunkGranularity,
    
    /// Maximum chunk size in characters (0 = no limit)
    pub max_chunk_size: usize,
    
    /// Include docstrings/comments in chunks
    pub include_docs: bool,
    
    /// Languages to process (None = auto-detect)
    pub languages: Option<Vec<Language>>,
}
```

### 2.4 Index Types (`index.rs`)

```rust
/// In-memory semantic index for fast similarity search
pub struct SemanticIndex {
    /// All embedded chunks
    chunks: Vec<EmbeddedChunk>,
    
    /// Model used for all embeddings
    model: EmbeddingModel,
    
    /// Project root path
    root: PathBuf,
}

/// Options for similarity search
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// Number of results to return
    pub top_k: usize,
    
    /// Minimum similarity threshold (0.0 to 1.0)
    pub threshold: f64,
    
    /// Model to use for query embedding
    pub model: EmbeddingModel,
    
    /// Exclude exact matches (for similarity search)
    pub exclude_self: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            top_k: 10,
            threshold: 0.5,
            model: EmbeddingModel::default(),
            exclude_self: false,
        }
    }
}
```

### 2.5 Cache Types (`cache.rs`)

```rust
/// JSON-based embedding cache
pub struct EmbeddingCache {
    /// Cache file path
    path: PathBuf,
    
    /// In-memory cache entries
    entries: HashMap<String, CacheEntry>,
    
    /// Dirty flag for lazy writes
    dirty: bool,
}

/// A single cache entry
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    /// Content hash of the source code
    content_hash: String,
    
    /// Model used to generate embedding
    model: EmbeddingModel,
    
    /// The embedding vector
    embedding: Vec<f32>,
    
    /// Timestamp when cached
    cached_at: u64,
}

/// Cache configuration
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Cache directory (default: ~/.cache/tldr/embeddings/)
    pub cache_dir: PathBuf,
    
    /// Maximum cache size in MB (default: 500)
    pub max_size_mb: usize,
    
    /// Cache entry TTL in days (default: 30)
    pub ttl_days: u32,
}
```

---

## 3. Core Components

### 3.1 Embedder (`embedder.rs`)

#### Primary Function

```rust
impl Embedder {
    /// Create a new embedder with the specified model
    ///
    /// # Arguments
    /// * `model` - The embedding model to use
    ///
    /// # Returns
    /// * `TldrResult<Self>` - Initialized embedder or error
    ///
    /// # Errors
    /// * `TldrError::ModelLoadError` - Failed to load model
    /// * `TldrError::IoError` - Cache directory inaccessible
    ///
    /// # Example
    /// ```rust
    /// let embedder = Embedder::new(EmbeddingModel::ArcticM)?;
    /// ```
    pub fn new(model: EmbeddingModel) -> TldrResult<Self>;
    
    /// Embed a single text
    ///
    /// # Arguments
    /// * `text` - Text to embed
    ///
    /// # Returns
    /// * `TldrResult<Vec<f32>>` - Embedding vector
    ///
    /// # Invariants
    /// * Output length == model.dimensions()
    /// * Output is normalized (L2 norm == 1.0)
    pub fn embed_text(&self, text: &str) -> TldrResult<Vec<f32>>;
    
    /// Embed multiple texts in batch
    ///
    /// # Arguments
    /// * `texts` - Texts to embed
    /// * `show_progress` - Whether to show progress bar
    ///
    /// # Returns
    /// * `TldrResult<Vec<Vec<f32>>>` - Embedding vectors
    ///
    /// # Performance
    /// * Batching reduces overhead for multiple texts
    /// * Default batch size: 32
    pub fn embed_batch(
        &self, 
        texts: &[String], 
        show_progress: bool
    ) -> TldrResult<Vec<Vec<f32>>>;
    
    /// Get the model being used
    pub fn model(&self) -> EmbeddingModel;
}
```

#### Behavioral Contract

| Input | Output | Notes |
|-------|--------|-------|
| Empty string | Zero vector | All zeros, normalized |
| Text > max_context | Truncated embedding | First N tokens embedded |
| Valid text | Normalized embedding | L2 norm == 1.0 |
| Invalid UTF-8 | Error | `TldrError::EncodingError` |

### 3.2 Chunker (`chunker.rs`)

#### Primary Function

```rust
/// Extract code chunks from a file or directory
///
/// # Arguments
/// * `path` - File or directory path
/// * `options` - Chunking options
///
/// # Returns
/// * `TldrResult<Vec<CodeChunk>>` - Extracted chunks
///
/// # Errors
/// * `TldrError::PathNotFound` - Path doesn't exist
/// * `TldrError::ParseError` - Syntax error in file
/// * `TldrError::UnsupportedLanguage` - No parser for file type
///
/// # Example
/// ```rust
/// let chunks = chunk_code(
///     Path::new("src/"),
///     ChunkOptions { granularity: ChunkGranularity::Function, ..Default::default() }
/// )?;
/// ```
pub fn chunk_code(path: &Path, options: ChunkOptions) -> TldrResult<Vec<CodeChunk>>;

/// Extract chunks from a single file
pub fn chunk_file(path: &Path, options: &ChunkOptions) -> TldrResult<Vec<CodeChunk>>;

/// Extract function-level chunks using tree-sitter
fn extract_function_chunks(
    tree: &Tree, 
    source: &str, 
    path: &Path,
    language: Language
) -> Vec<CodeChunk>;
```

#### Behavioral Contract

| Input | Granularity | Output |
|-------|-------------|--------|
| Single file | File | 1 chunk (whole file) |
| Single file | Function | N chunks (one per function/method) |
| Directory | Function | All functions from all files |
| Empty file | Any | 0 chunks |
| Binary file | Any | Skipped (0 chunks) |
| Parse error | Any | File skipped with warning |

#### Integration with Existing AST Module

The chunker reuses `tldr_core::ast::extractor` for function extraction:

```rust
use crate::ast::extractor::{extract_functions, extract_methods, extract_classes};
use crate::ast::parser::parse_file;

fn extract_function_chunks(path: &Path, language: Language) -> TldrResult<Vec<CodeChunk>> {
    let (tree, source, _) = parse_file(path)?;
    
    // Reuse existing function extraction
    let functions = extract_functions(&tree, &source, language);
    let methods = extract_methods(&tree, &source, language);
    
    // Build chunks with line ranges
    // ...
}
```

### 3.3 Index (`index.rs`)

#### Primary Functions

```rust
impl SemanticIndex {
    /// Build an index from a project directory
    ///
    /// # Arguments
    /// * `root` - Project root directory
    /// * `options` - Chunking and embedding options
    /// * `cache` - Optional embedding cache
    ///
    /// # Returns
    /// * `TldrResult<Self>` - Built index
    pub fn build(
        root: &Path,
        chunk_options: ChunkOptions,
        embed_options: EmbedOptions,
        cache: Option<&mut EmbeddingCache>,
    ) -> TldrResult<Self>;
    
    /// Search for chunks matching a natural language query
    ///
    /// # Arguments
    /// * `query` - Natural language search query
    /// * `options` - Search options
    ///
    /// # Returns
    /// * `TldrResult<SemanticSearchReport>` - Search results
    ///
    /// # Example
    /// ```rust
    /// let index = SemanticIndex::build(root, opts, embed_opts, None)?;
    /// let results = index.search("parse configuration file", SearchOptions::default())?;
    /// ```
    pub fn search(&self, query: &str, options: SearchOptions) -> TldrResult<SemanticSearchReport>;
    
    /// Find chunks similar to a given chunk
    ///
    /// # Arguments
    /// * `chunk` - Source chunk to find similar code for
    /// * `options` - Search options
    ///
    /// # Returns
    /// * `TldrResult<SimilarityReport>` - Similar chunks
    pub fn find_similar(
        &self,
        chunk: &CodeChunk,
        options: SearchOptions,
    ) -> TldrResult<SimilarityReport>;
    
    /// Get a chunk by file path and function name
    pub fn get_chunk(&self, file: &Path, function: Option<&str>) -> Option<&EmbeddedChunk>;
    
    /// Get total number of chunks in the index
    pub fn len(&self) -> usize;
    
    /// Check if index is empty
    pub fn is_empty(&self) -> bool;
}
```

### 3.4 Similarity (`similarity.rs`)

#### Primary Functions

```rust
/// Compute cosine similarity between two vectors
///
/// # Arguments
/// * `a` - First vector (must be normalized)
/// * `b` - Second vector (must be normalized)
///
/// # Returns
/// * `f64` - Cosine similarity (0.0 to 1.0 for normalized vectors)
///
/// # Panics
/// * If vectors have different lengths
///
/// # Performance
/// * O(n) where n = vector dimension
/// * ~768 multiplications for ArcticM
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64;

/// Find top-K most similar vectors using linear scan
///
/// # Arguments
/// * `query` - Query vector
/// * `candidates` - Candidate vectors to search
/// * `k` - Number of results to return
/// * `threshold` - Minimum similarity threshold
///
/// # Returns
/// * `Vec<(usize, f64)>` - (index, score) pairs sorted by score descending
///
/// # Performance
/// * O(n * d) where n = candidates, d = dimensions
/// * ~7.68M operations for 10K functions with 768-dim embeddings
pub fn top_k_similar(
    query: &[f32],
    candidates: &[Vec<f32>],
    k: usize,
    threshold: f64,
) -> Vec<(usize, f64)>;

/// Normalize a vector to unit length (L2 norm = 1)
pub fn normalize(v: &mut [f32]);

/// Check if a vector is normalized (L2 norm ≈ 1.0)
pub fn is_normalized(v: &[f32]) -> bool;
```

### 3.5 Cache (`cache.rs`)

#### Primary Functions

```rust
impl EmbeddingCache {
    /// Open or create a cache at the given directory
    ///
    /// # Arguments
    /// * `config` - Cache configuration
    ///
    /// # Returns
    /// * `TldrResult<Self>` - Cache instance
    pub fn open(config: CacheConfig) -> TldrResult<Self>;
    
    /// Get cached embedding for a chunk
    ///
    /// # Arguments
    /// * `chunk` - Code chunk to look up
    /// * `model` - Model that was used
    ///
    /// # Returns
    /// * `Option<Vec<f32>>` - Cached embedding if valid
    ///
    /// # Cache Key
    /// Key = SHA256(file_path + content_hash + model_name)
    pub fn get(&self, chunk: &CodeChunk, model: EmbeddingModel) -> Option<Vec<f32>>;
    
    /// Store embedding in cache
    ///
    /// # Arguments
    /// * `chunk` - Code chunk
    /// * `embedding` - Embedding vector
    /// * `model` - Model used
    pub fn put(&mut self, chunk: &CodeChunk, embedding: Vec<f32>, model: EmbeddingModel);
    
    /// Flush dirty entries to disk
    pub fn flush(&mut self) -> TldrResult<()>;
    
    /// Evict stale entries older than TTL
    pub fn evict_stale(&mut self) -> usize;
    
    /// Get cache statistics
    pub fn stats(&self) -> CacheStats;
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheStats {
    /// Number of entries
    pub entries: usize,
    /// Total size in bytes
    pub size_bytes: usize,
    /// Hit rate (0.0 to 1.0)
    pub hit_rate: f64,
}
```

#### Cache File Format

```json
{
  "version": 1,
  "model": "arctic-m",
  "entries": {
    "sha256:abc123...": {
      "content_hash": "md5:def456...",
      "embedding": [0.123, -0.456, ...],
      "cached_at": 1706918400
    }
  }
}
```

---

## 4. CLI Commands

### 4.1 `tldr semantic` Command

```rust
/// Semantic code search using embeddings
#[derive(Debug, Args)]
pub struct SemanticArgs {
    /// Natural language search query
    pub query: String,
    
    /// Path to search (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,
    
    /// Number of results to return
    #[arg(long, short = 'n', default_value = "10")]
    pub top: usize,
    
    /// Minimum similarity threshold (0.0 to 1.0)
    #[arg(long, short = 't', default_value = "0.5")]
    pub threshold: f64,
    
    /// Embedding model (xs, s, m, m-long, l)
    #[arg(long, short = 'm', default_value = "m")]
    pub model: EmbeddingModel,
    
    /// Programming language (auto-detect if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
    
    /// Disable cache
    #[arg(long)]
    pub no_cache: bool,
}
```

#### Example Usage

```bash
# Basic semantic search
$ tldr semantic "parse config file" src/

# With options
$ tldr semantic "error handling retry logic" --top 5 --threshold 0.7 --model m

# Specific language
$ tldr semantic "database connection pool" --lang python src/
```

#### Example Output (JSON)

```json
{
  "results": [
    {
      "file_path": "src/config.rs",
      "function_name": "parse_config",
      "class_name": null,
      "score": 0.89,
      "line_start": 10,
      "line_end": 45,
      "snippet": "fn parse_config(path: &Path) -> Result<Config> {"
    },
    {
      "file_path": "src/loader.rs",
      "function_name": "load_config",
      "class_name": null,
      "score": 0.82,
      "line_start": 20,
      "line_end": 35,
      "snippet": "fn load_config() -> Config {"
    }
  ],
  "query": "parse config file",
  "model": "arctic-m",
  "total_chunks": 150,
  "matches_above_threshold": 8,
  "latency_ms": 245,
  "cache_hit": true
}
```

#### Example Output (Text)

```
Semantic search: "parse config file"
Model: arctic-m | Threshold: 0.50 | Searched: 150 chunks

Results (8 matches):

1. src/config.rs:parse_config (score: 0.89)
   Lines 10-45
   fn parse_config(path: &Path) -> Result<Config> {

2. src/loader.rs:load_config (score: 0.82)
   Lines 20-35
   fn load_config() -> Config {

3. src/settings.rs:read_settings (score: 0.71)
   Lines 5-22
   fn read_settings(file: &str) -> Settings {

Search completed in 245ms (cache hit)
```

### 4.2 `tldr embed` Command

```rust
/// Generate embeddings for code
#[derive(Debug, Args)]
pub struct EmbedArgs {
    /// File or directory to embed
    pub path: PathBuf,
    
    /// Output file (stdout if not specified)
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,
    
    /// Chunking granularity (file or function)
    #[arg(long, short = 'g', default_value = "function")]
    pub granularity: ChunkGranularity,
    
    /// Embedding model
    #[arg(long, short = 'm', default_value = "m")]
    pub model: EmbeddingModel,
    
    /// Programming language
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
    
    /// Include embedding vectors in output
    #[arg(long)]
    pub include_vectors: bool,
}
```

#### Example Usage

```bash
# Embed a file
$ tldr embed src/config.rs

# Embed a directory with function-level chunks
$ tldr embed src/ --granularity function --output embeddings.json

# Include vectors in output
$ tldr embed src/config.rs --include-vectors
```

#### Example Output (JSON)

```json
{
  "path": "src/config.rs",
  "model": "arctic-m",
  "granularity": "function",
  "chunks_embedded": 5,
  "chunks_cached": 3,
  "latency_ms": 120,
  "chunks": [
    {
      "chunk": {
        "file_path": "src/config.rs",
        "function_name": "parse_config",
        "class_name": null,
        "line_start": 10,
        "line_end": 45,
        "content": "fn parse_config(path: &Path) -> Result<Config> {...}",
        "content_hash": "md5:abc123",
        "language": "rust"
      },
      "embedding": [0.123, -0.456, ...],
      "model": "arctic-m",
      "embedded_at": 1706918400
    }
  ]
}
```

### 4.3 `tldr similar` Command

```rust
/// Find similar code fragments
#[derive(Debug, Args)]
pub struct SimilarArgs {
    /// Source file to find similar code for
    pub file: PathBuf,
    
    /// Specific function to compare (whole file if not specified)
    #[arg(long, short = 'f')]
    pub function: Option<String>,
    
    /// Number of similar results
    #[arg(long, short = 'n', default_value = "5")]
    pub top: usize,
    
    /// Minimum similarity threshold
    #[arg(long, short = 't', default_value = "0.7")]
    pub threshold: f64,
    
    /// Exclude self from results
    #[arg(long, default_value = "true")]
    pub exclude_self: bool,
    
    /// Search path (default: current directory)
    #[arg(long, short = 'p', default_value = ".")]
    pub path: PathBuf,
    
    /// Embedding model
    #[arg(long, short = 'm', default_value = "m")]
    pub model: EmbeddingModel,
}
```

#### Example Usage

```bash
# Find code similar to a function
$ tldr similar src/config.rs --function parse_config

# Find similar files
$ tldr similar src/config.rs --top 10 --threshold 0.6

# Search in specific directory
$ tldr similar src/config.rs --function parse_config --path lib/
```

#### Example Output (JSON)

```json
{
  "source": {
    "file_path": "src/config.rs",
    "function_name": "parse_config",
    "line_start": 10,
    "line_end": 45
  },
  "similar": [
    {
      "file_path": "src/loader.rs",
      "function_name": "load_config",
      "score": 0.85,
      "line_start": 20,
      "line_end": 35,
      "snippet": "fn load_config() -> Config {"
    },
    {
      "file_path": "src/settings.rs", 
      "function_name": "read_settings",
      "score": 0.72,
      "line_start": 5,
      "line_end": 22,
      "snippet": "fn read_settings(file: &str) -> Settings {"
    }
  ],
  "model": "arctic-m",
  "total_compared": 150,
  "exclude_self": true
}
```

### 4.4 `tldr explain` Command (DEFERRED)

```rust
/// LLM-powered code explanation
#[derive(Debug, Args)]
pub struct ExplainArgs {
    /// File containing the function
    pub file: PathBuf,
    
    /// Function name to explain
    pub function: String,
    
    /// Detail level
    #[arg(long, short = 'd', default_value = "brief")]
    pub detail: DetailLevel,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum DetailLevel {
    /// One-sentence summary
    Brief,
    /// Full explanation with examples
    Full,
}
```

**Note:** This command is deferred to a future session as it requires LLM integration.

---

## 5. Behavioral Contracts

### 5.1 Embedding Invariants

| Property | Invariant |
|----------|-----------|
| Normalization | All embeddings have L2 norm = 1.0 (tolerance: 1e-6) |
| Dimensions | Output dimensions match model.dimensions() exactly |
| Determinism | Same input text + model produces identical embeddings |
| Empty input | Returns zero vector (normalized) |

### 5.2 Similarity Invariants

| Property | Invariant |
|----------|-----------|
| Range | Cosine similarity ∈ [-1.0, 1.0] |
| Self-similarity | A · A = 1.0 for normalized vectors |
| Symmetry | A · B = B · A |
| Triangle inequality | Not guaranteed (embeddings are semantic, not metric) |

### 5.3 Caching Invariants

| Property | Invariant |
|----------|-----------|
| Key uniqueness | Key = hash(file_path + content_hash + model) |
| Invalidation | Cache miss if content_hash differs |
| Model isolation | Different models have separate cache entries |
| Concurrent safety | Cache file locked during writes |

### 5.4 Search Invariants

| Property | Invariant |
|----------|-----------|
| Ordering | Results sorted by score descending |
| Threshold | All results have score >= threshold |
| Top-K | At most K results returned |
| Completeness | All chunks above threshold are candidates |

---

## 6. Error Handling

### 6.1 New Error Types

Add to `tldr_core::error::TldrError`:

```rust
// =========================================================================
// Semantic Search Errors (Session 16)
// =========================================================================

/// Model loading/initialization failed
#[error("Failed to load embedding model '{model}': {detail}")]
ModelLoadError {
    model: String,
    detail: String,
},

/// Embedding generation failed
#[error("Embedding failed for {file}: {detail}")]
EmbeddingError {
    file: PathBuf,
    detail: String,
},

/// Cache corruption or version mismatch
#[error("Embedding cache corrupted: {0}")]
CacheCorrupted(String),

/// No chunks found to embed
#[error("No embeddable code chunks found in {0}")]
NoChunksFound(PathBuf),

/// Index not built
#[error("Semantic index not initialized. Run 'tldr embed' first.")]
IndexNotBuilt,
```

### 6.2 Error Scenarios

| Scenario | Error | Recovery |
|----------|-------|----------|
| Model not downloaded | `ModelLoadError` | Automatic download on first use |
| ONNX runtime failure | `ModelLoadError` | User retry with --offline |
| Parse error in file | `ParseError` | Skip file, continue processing |
| Cache file corrupted | `CacheCorrupted` | Delete and rebuild cache |
| Empty directory | `NoChunksFound` | Return empty results |
| Out of memory | `EmbeddingError` | Reduce batch size |

### 6.3 Graceful Degradation

Following the existing M8 pattern from `embedding_client.rs`:

```rust
/// Perform semantic search with graceful degradation
pub fn search_with_fallback(
    query: &str,
    root: &Path,
    options: SearchOptions,
) -> TldrResult<SemanticSearchReport> {
    match SemanticIndex::build(root, ChunkOptions::default(), EmbedOptions::default(), None) {
        Ok(index) => index.search(query, options),
        Err(TldrError::ModelLoadError { .. }) => {
            // Fall back to BM25 search
            eprintln!("Warning: Embedding model unavailable, falling back to keyword search");
            let bm25_results = Bm25Index::from_project(root, Language::Python)?
                .search(query, options.top_k);
            // Convert BM25 results to SemanticSearchReport format
            Ok(convert_bm25_to_semantic(bm25_results, query))
        }
        Err(e) => Err(e),
    }
}
```

---

## 7. Integration Points

### 7.1 With Existing AST Module

The semantic module reuses tree-sitter parsing from `tldr_core::ast`:

```rust
// In chunker.rs
use crate::ast::parser::parse_file;
use crate::ast::extractor::{extract_functions, extract_methods};

pub fn chunk_file(path: &Path, options: &ChunkOptions) -> TldrResult<Vec<CodeChunk>> {
    let (tree, source, _) = parse_file(path)?;
    let language = Language::from_path(path).ok_or_else(|| {
        TldrError::UnsupportedLanguage(path.extension().unwrap_or_default().to_string_lossy().to_string())
    })?;
    
    match options.granularity {
        ChunkGranularity::File => {
            Ok(vec![CodeChunk {
                file_path: path.to_path_buf(),
                function_name: None,
                class_name: None,
                line_start: 1,
                line_end: source.lines().count() as u32,
                content: source.to_string(),
                content_hash: hash_content(&source),
                language,
            }])
        }
        ChunkGranularity::Function => {
            extract_function_chunks_from_tree(&tree, &source, path, language)
        }
    }
}
```

### 7.2 With Existing Search Module

The semantic module complements the existing `search` module:

```rust
// In lib.rs
pub mod search;     // BM25, hybrid search
pub mod semantic;   // Dense embeddings, semantic search

// Re-exports
pub use search::{Bm25Index, hybrid_search, HybridResult};
pub use semantic::{SemanticIndex, SemanticSearchResult, Embedder, EmbeddingModel};
```

### 7.3 With Existing CLI Patterns

Commands follow the established pattern from `imports.rs`, `cfg.rs`, etc.:

```rust
// In semantic.rs
impl SemanticArgs {
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);
        
        // Auto-detect language if not specified
        let language = self.lang.or_else(|| {
            Language::from_path(&self.path)
        });
        
        writer.progress(&format!(
            "Building semantic index for {}...",
            self.path.display()
        ));
        
        // Build index
        let cache = if self.no_cache {
            None
        } else {
            Some(EmbeddingCache::open(CacheConfig::default())?)
        };
        
        let index = SemanticIndex::build(
            &self.path,
            ChunkOptions { 
                granularity: ChunkGranularity::Function,
                ..Default::default()
            },
            EmbedOptions { model: self.model, ..Default::default() },
            cache.as_mut(),
        )?;
        
        writer.progress(&format!(
            "Searching {} chunks for '{}'...",
            index.len(),
            self.query
        ));
        
        // Search
        let report = index.search(
            &self.query,
            SearchOptions {
                top_k: self.top,
                threshold: self.threshold,
                model: self.model,
                ..Default::default()
            },
        )?;
        
        // Output
        if writer.is_text() {
            let text = format_semantic_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }
        
        Ok(())
    }
}
```

### 7.4 With Hybrid Search

The semantic module can be integrated with the existing hybrid search:

```rust
// Future enhancement: Hybrid RRF with semantic embeddings
pub fn hybrid_semantic_search(
    query: &str,
    root: &Path,
    options: HybridSearchOptions,
) -> TldrResult<HybridSearchReport> {
    // BM25 results
    let bm25_index = Bm25Index::from_project(root, options.language)?;
    let bm25_results = bm25_index.search(query, options.top_k * 2);
    
    // Semantic results
    let semantic_index = SemanticIndex::build(root, ...)?;
    let semantic_results = semantic_index.search(query, ...)?;
    
    // Fuse with RRF
    fuse_rrf(&bm25_results, &semantic_results.results, options.k_constant, options.top_k)
}
```

---

## 8. Performance Considerations

### 8.1 Time Complexity

| Operation | Complexity | Typical Time |
|-----------|------------|--------------|
| Model load | O(1) | ~2s (first time), ~100ms (cached) |
| Embed single text | O(d) | ~10ms |
| Embed batch (n texts) | O(n * d / batch) | ~50ms for 100 texts |
| Build index (n files) | O(n * (parse + embed)) | ~5s for 1000 functions |
| Search (n chunks) | O(n * d) | ~10ms for 10K chunks |
| Cache lookup | O(1) | ~1ms |

### 8.2 Space Complexity

| Component | Size |
|-----------|------|
| Model (ArcticM) | ~110MB on disk |
| Single embedding | 768 * 4 = 3KB |
| 10K embeddings | ~30MB in memory |
| Cache entry | ~3.5KB per chunk |
| Cache (10K) | ~35MB on disk |

### 8.3 Optimization Strategies

1. **Lazy model loading**: Model loaded on first embed call
2. **Batch embedding**: Process multiple chunks in single forward pass
3. **Cache-first**: Check cache before embedding
4. **Incremental updates**: Only re-embed changed files
5. **Parallel chunking**: Use rayon for file processing

### 8.4 Memory Limits

```rust
/// Maximum chunks to hold in memory
pub const MAX_INDEX_SIZE: usize = 100_000;

/// Maximum batch size for embedding
pub const MAX_BATCH_SIZE: usize = 64;

/// Maximum file size to embed (10MB)
pub const MAX_FILE_SIZE: usize = 10 * 1024 * 1024;
```

---

## 9. Testing Strategy

### 9.1 Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![0.5, 0.5, 0.5, 0.5];
        let normalized = normalize(&v);
        assert!((cosine_similarity(&normalized, &normalized) - 1.0).abs() < 1e-6);
    }
    
    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &b)).abs() < 1e-6);
    }
    
    #[test]
    fn test_chunk_file_granularity() {
        let tmp = create_test_file("fn foo() {}\nfn bar() {}");
        let chunks = chunk_file(&tmp, &ChunkOptions {
            granularity: ChunkGranularity::Function,
            ..Default::default()
        }).unwrap();
        assert_eq!(chunks.len(), 2);
    }
    
    #[test]
    fn test_cache_invalidation() {
        let mut cache = EmbeddingCache::open(CacheConfig::default()).unwrap();
        let chunk1 = CodeChunk { content_hash: "abc".into(), ..};
        let chunk2 = CodeChunk { content_hash: "xyz".into(), ..};
        
        cache.put(&chunk1, vec![0.1; 768], EmbeddingModel::ArcticM);
        assert!(cache.get(&chunk1, EmbeddingModel::ArcticM).is_some());
        assert!(cache.get(&chunk2, EmbeddingModel::ArcticM).is_none());
    }
    
    #[test]
    fn test_top_k_ordering() {
        let query = vec![1.0, 0.0];
        let candidates = vec![
            vec![0.9, 0.1],  // high similarity
            vec![0.1, 0.9],  // low similarity
            vec![0.7, 0.3],  // medium similarity
        ];
        
        let results = top_k_similar(&query, &candidates, 3, 0.0);
        assert_eq!(results[0].0, 0); // highest first
        assert_eq!(results[1].0, 2);
        assert_eq!(results[2].0, 1);
    }
}
```

### 9.2 Integration Tests

```rust
#[test]
fn test_semantic_search_e2e() {
    let tmp_dir = tempdir().unwrap();
    
    // Create test files
    fs::write(tmp_dir.path().join("config.rs"), "fn parse_config() {}").unwrap();
    fs::write(tmp_dir.path().join("loader.rs"), "fn load_settings() {}").unwrap();
    
    // Build index and search
    let index = SemanticIndex::build(
        tmp_dir.path(),
        ChunkOptions::default(),
        EmbedOptions::default(),
        None,
    ).unwrap();
    
    let results = index.search("configuration parser", SearchOptions::default()).unwrap();
    
    // parse_config should rank higher than load_settings
    assert!(!results.results.is_empty());
    assert!(results.results[0].function_name.as_ref().unwrap().contains("config"));
}
```

### 9.3 Benchmark Tests

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_similarity_search(c: &mut Criterion) {
    let query = vec![0.5; 768];
    let candidates: Vec<Vec<f32>> = (0..10000)
        .map(|_| (0..768).map(|_| rand::random()).collect())
        .collect();
    
    c.bench_function("top_k_10k_chunks", |b| {
        b.iter(|| top_k_similar(black_box(&query), black_box(&candidates), 10, 0.5))
    });
}

criterion_group!(benches, bench_similarity_search);
criterion_main!(benches);
```

---

## 10. Dependencies

### 10.1 New Cargo Dependencies

Add to `tldr-core/Cargo.toml`:

```toml
[dependencies]
# Semantic search (Session 16)
fastembed = "5.8"

# Content hashing for cache keys
md5 = "0.7"
sha2 = "0.10"

# Progress reporting
indicatif = { version = "0.17", optional = true }

[features]
default = []
progress = ["indicatif"]
```

### 10.2 Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `progress` | Show progress bars during embedding | Off |
| `cache` | Enable embedding cache | On |

---

## 11. Migration Path

### Phase 1: Foundation (This Session)
- [ ] Create `semantic/` module structure
- [ ] Implement `types.rs` with all types
- [ ] Implement `embedder.rs` with fastembed wrapper
- [ ] Implement `similarity.rs` with cosine similarity

### Phase 2: Core Features
- [ ] Implement `chunker.rs` with tree-sitter integration
- [ ] Implement `index.rs` with in-memory search
- [ ] Implement `cache.rs` with JSON persistence

### Phase 3: CLI Commands
- [ ] Implement `semantic` command
- [ ] Implement `embed` command  
- [ ] Implement `similar` command

### Phase 4: Integration
- [ ] Add to `lib.rs` exports
- [ ] Add to CLI command registry
- [ ] Update `--help` documentation
- [ ] Add integration tests

### Phase 5: Optimization (Future)
- [ ] SPLADE sparse embeddings (hybrid search)
- [ ] Cross-encoder reranking
- [ ] Incremental index updates
- [ ] `explain` command with LLM

---

## 12. Open Questions

1. **Model download behavior**: Should first-time model download be automatic or require explicit `--download` flag?
   - **Decision**: Automatic with warning message

2. **Cache location**: Should cache be per-project or global?
   - **Decision**: Global (`~/.cache/tldr/embeddings/`) with project-specific entries

3. **Hybrid search integration**: Should semantic search automatically include BM25?
   - **Decision**: Separate commands; hybrid search is future enhancement

4. **Large codebase handling**: What's the cutoff for in-memory vs. external index?
   - **Decision**: 100K chunks in-memory; beyond that, warn and suggest filtering

---

## 13. Success Criteria

The semantic search module is complete when:

1. **Functional**: All three commands (`semantic`, `embed`, `similar`) work end-to-end
2. **Correct**: Similarity scores are accurate and rankings are sensible
3. **Performant**: Search completes in <1s for 10K function index
4. **Integrated**: Commands follow existing CLI patterns and output formats
5. **Tested**: >80% code coverage with unit and integration tests
6. **Documented**: All public APIs have doc comments with examples

---

*End of Specification*
