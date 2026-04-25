//! BM25 keyword search implementation
//!
//! Implements BM25 (Best Matching 25) ranking algorithm for code search.
//! Uses code-aware tokenization for camelCase/snake_case splitting.
//!
//! # BM25 Formula
//! ```text
//! score(D, Q) = sum(IDF(qi) * (tf * (k1 + 1)) / (tf + k1 * (1 - b + b * |D|/avgdl)))
//! ```
//!
//! Where:
//! - tf: term frequency in document
//! - IDF: inverse document frequency
//! - k1: term frequency saturation parameter (default 1.5)
//! - b: document length normalization parameter (default 0.75)
//! - avgdl: average document length

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use super::tokenizer::Tokenizer;
use crate::fs::tree::DEFAULT_SKIP_DIRS;
use crate::types::Language;
use crate::TldrResult;

/// BM25 search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25Result {
    /// File path
    pub file_path: PathBuf,
    /// BM25 relevance score
    pub score: f64,
    /// Start line of the matching region
    pub line_start: u32,
    /// End line of the matching region
    pub line_end: u32,
    /// Snippet of matching content
    pub snippet: String,
    /// Terms that matched in this document
    pub matched_terms: Vec<String>,
}

/// Document in the BM25 index
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Document {
    /// Document ID (file path)
    id: String,
    /// Term frequencies
    term_freqs: HashMap<String, u32>,
    /// Total number of tokens
    length: usize,
    /// Original content for snippet extraction
    content: String,
}

/// BM25 search index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25Index {
    /// k1 parameter: term frequency saturation (default 1.5)
    k1: f64,
    /// b parameter: document length normalization (default 0.75)
    b: f64,
    /// All indexed documents
    documents: Vec<Document>,
    /// Document frequency for each term (how many docs contain term)
    doc_freqs: HashMap<String, usize>,
    /// Average document length
    avg_doc_length: f64,
    /// Running sum of all document lengths (integer to avoid float drift).
    /// INVARIANT: Must be recalculated if documents are ever removed.
    total_doc_length: usize,
    /// Tokenizer instance
    tokenizer: Tokenizer,
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self::new(1.5, 0.75)
    }
}

impl Bm25Index {
    /// Create a new BM25 index with specified parameters
    ///
    /// # Arguments
    /// * `k1` - Term frequency saturation (default 1.5, higher = more weight to term frequency)
    /// * `b` - Document length normalization (default 0.75, 0 = no normalization, 1 = full normalization)
    pub fn new(k1: f64, b: f64) -> Self {
        Self {
            k1,
            b,
            documents: Vec::new(),
            doc_freqs: HashMap::new(),
            avg_doc_length: 0.0,
            total_doc_length: 0,
            tokenizer: Tokenizer::new(),
        }
    }

    /// Add a document to the index
    ///
    /// # Arguments
    /// * `doc_id` - Unique identifier for the document (typically file path)
    /// * `content` - Text content to index
    pub fn add_document(&mut self, doc_id: &str, content: &str) {
        let tokens = self.tokenizer.tokenize(content);
        let length = tokens.len();

        // Count term frequencies
        let mut term_freqs: HashMap<String, u32> = HashMap::new();
        let mut unique_terms: HashSet<String> = HashSet::new();

        for token in &tokens {
            *term_freqs.entry(token.clone()).or_insert(0) += 1;
            unique_terms.insert(token.clone());
        }

        // Update document frequencies
        for term in unique_terms {
            *self.doc_freqs.entry(term).or_insert(0) += 1;
        }

        // Add document
        self.documents.push(Document {
            id: doc_id.to_string(),
            term_freqs,
            length,
            content: content.to_string(),
        });

        // Update average document length in O(1) instead of O(n)
        self.total_doc_length += length;
        self.avg_doc_length = self.total_doc_length as f64 / self.documents.len() as f64;
    }

    /// Search the index for relevant documents
    ///
    /// # Arguments
    /// * `query` - Search query string
    /// * `top_k` - Maximum number of results to return
    ///
    /// # Returns
    /// Vector of search results sorted by relevance score (descending)
    pub fn search(&self, query: &str, top_k: usize) -> Vec<Bm25Result> {
        let query_tokens = self.tokenizer.tokenize(query);

        if query_tokens.is_empty() || self.documents.is_empty() {
            return Vec::new();
        }

        let n = self.documents.len() as f64;

        // Score each document
        let mut scores: Vec<(usize, f64, Vec<String>)> = Vec::new();

        for (doc_idx, doc) in self.documents.iter().enumerate() {
            let mut score = 0.0;
            let mut matched_terms = Vec::new();

            for term in &query_tokens {
                let tf = *doc.term_freqs.get(term).unwrap_or(&0) as f64;

                if tf > 0.0 {
                    matched_terms.push(term.clone());

                    // IDF calculation
                    let df = *self.doc_freqs.get(term).unwrap_or(&0) as f64;
                    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();

                    // BM25 score component
                    let doc_len = doc.length as f64;
                    let numerator = tf * (self.k1 + 1.0);
                    let denominator =
                        tf + self.k1 * (1.0 - self.b + self.b * doc_len / self.avg_doc_length);

                    score += idf * (numerator / denominator);
                }
            }

            if score > 0.0 {
                scores.push((doc_idx, score, matched_terms));
            }
        }

        // Sort by score descending
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Convert to results
        scores
            .into_iter()
            .take(top_k)
            .map(|(idx, score, matched_terms)| {
                let doc = &self.documents[idx];
                let (line_start, line_end, snippet) = extract_snippet(&doc.content, &matched_terms);

                Bm25Result {
                    file_path: PathBuf::from(&doc.id),
                    score,
                    line_start,
                    line_end,
                    snippet,
                    matched_terms,
                }
            })
            .collect()
    }

    /// Build an index from all code files in a project directory
    ///
    /// # Arguments
    /// * `root` - Root directory to index
    /// * `language` - Language to filter by (only index files of this language)
    pub fn from_project(root: &Path, language: Language) -> TldrResult<Self> {
        let mut index = Self::default();
        let extensions: HashSet<&str> = language.extensions().iter().copied().collect();

        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                // VAL-018: never reject the WalkDir root (depth 0). The
                // user-supplied root may legitimately have a leading dot
                // (e.g. `.tmpXXXXXX` from tempfile, or any path under a
                // hidden parent). Filtering by depth 0 keeps the
                // hidden-skip semantics for descendants while not
                // silently producing 0 results when the project root
                // itself starts with `.`.
                if e.depth() == 0 {
                    return true;
                }
                let name = e.file_name().to_string_lossy();
                // Skip hidden and default skip directories below the root.
                if name.starts_with('.') && name != "." {
                    return false;
                }
                if e.file_type().is_dir() && DEFAULT_SKIP_DIRS.contains(&name.as_ref()) {
                    return false;
                }
                true
            })
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            // Skip directories
            if entry.file_type().is_dir() {
                continue;
            }

            // Check extension
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{}", e));

            if let Some(ext) = &ext {
                if !extensions.contains(ext.as_str()) {
                    continue;
                }
            } else {
                continue;
            }

            // Read and index file
            if let Ok(content) = fs::read_to_string(path) {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();

                index.add_document(&relative, &content);
            }
        }

        Ok(index)
    }

    /// Get the number of documents in the index
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    /// Check if the index is empty
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
}

/// Extract a relevant snippet from content based on matched terms
fn extract_snippet(content: &str, matched_terms: &[String]) -> (u32, u32, String) {
    let lines: Vec<&str> = content.lines().collect();

    if lines.is_empty() {
        return (1, 1, String::new());
    }

    // Find the line with the most matched terms
    let mut best_line_idx = 0;
    let mut best_score = 0;

    for (idx, line) in lines.iter().enumerate() {
        let line_lower = line.to_lowercase();
        let score = matched_terms
            .iter()
            .filter(|term| line_lower.contains(term.as_str()))
            .count();

        if score > best_score {
            best_score = score;
            best_line_idx = idx;
        }
    }

    // Get context around best line (3 lines total)
    let start = best_line_idx.saturating_sub(1);
    let end = (best_line_idx + 2).min(lines.len());

    let snippet = lines[start..end].join("\n");

    ((start + 1) as u32, end as u32, snippet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bm25_add_document() {
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "def process_data items");
        index.add_document("file2", "class DataProcessor");

        assert_eq!(index.document_count(), 2);
    }

    #[test]
    fn test_bm25_search_basic() {
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "process data items data data");
        index.add_document("file2", "process something else");

        let results = index.search("data", 10);
        assert!(!results.is_empty());
        // file1 should rank higher (more occurrences of "data")
        assert_eq!(results[0].file_path, PathBuf::from("file1"));
    }

    #[test]
    fn test_bm25_returns_scores() {
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "process data");

        let results = index.search("data", 10);
        assert!(!results.is_empty());
        assert!(results[0].score > 0.0);
    }

    #[test]
    fn test_bm25_returns_matched_terms() {
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "process user data");

        let results = index.search("process data", 10);
        assert!(!results.is_empty());
        assert!(results[0].matched_terms.contains(&"process".to_string()));
        assert!(results[0].matched_terms.contains(&"data".to_string()));
    }

    #[test]
    fn test_bm25_respects_top_k() {
        let mut index = Bm25Index::new(1.5, 0.75);
        for i in 0..10 {
            index.add_document(&format!("file{}", i), "process data");
        }

        let results = index.search("data", 5);
        assert!(results.len() <= 5);
    }

    #[test]
    fn test_bm25_tokenizes_camel_case() {
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "processData ItemProcessor");

        let results = index.search("process", 10);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_bm25_tokenizes_snake_case() {
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "process_data item_processor");

        let results = index.search("process", 10);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_bm25_case_insensitive() {
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "PROCESS_DATA");

        let results = index.search("process", 10);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_bm25_empty_query() {
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "process data");

        let results = index.search("", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_bm25_no_match() {
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "process data");

        let results = index.search("nonexistent", 10);
        assert!(results.is_empty());
    }
}
