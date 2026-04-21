//! HTTP client for Python embedding service
//!
//! Provides semantic search embeddings by calling an external Python service.
//! The service should expose a REST API for generating embeddings.
//!
//! # Mitigation M8
//! This client implements graceful degradation - if the embedding service
//! is unavailable, hybrid search falls back to BM25-only mode.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::TldrError;
use crate::TldrResult;

/// Default embedding service URL
pub const DEFAULT_EMBEDDING_URL: &str = "http://localhost:8765";

/// Default timeout for embedding requests
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Embedding vector dimension (for BGE-large-en-v1.5)
pub const EMBEDDING_DIM: usize = 1024;

/// Request to the embedding service
#[derive(Debug, Clone, Serialize)]
struct EmbeddingRequest {
    /// Text to embed
    text: String,
    /// Optional batch of texts
    #[serde(skip_serializing_if = "Option::is_none")]
    texts: Option<Vec<String>>,
}

/// Response from the embedding service
#[derive(Debug, Clone, Deserialize)]
struct _EmbeddingResponse {
    /// Single embedding vector
    #[serde(default)]
    _embedding: Vec<f32>,
    /// Batch of embedding vectors
    #[serde(default)]
    _embeddings: Vec<Vec<f32>>,
}

/// Semantic search result from embedding service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticResult {
    /// Document ID / file path
    pub doc_id: String,
    /// Cosine similarity score (0-1)
    pub score: f64,
    /// Start line of matching region
    pub line_start: u32,
    /// End line of matching region
    pub line_end: u32,
    /// Snippet of content
    pub snippet: String,
}

/// Search request to the embedding service
#[derive(Debug, Clone, Serialize)]
struct SearchRequest {
    /// Query text
    query: String,
    /// Number of results to return
    top_k: usize,
    /// Project path to search in
    project: String,
}

/// Search response from embedding service
#[derive(Debug, Clone, Deserialize)]
struct _SearchResponse {
    /// Search results
    _results: Vec<SemanticResult>,
}

/// Client for the Python embedding service
///
/// # Example
/// ```ignore
/// use tldr_core::search::embedding_client::EmbeddingClient;
///
/// let client = EmbeddingClient::new("http://localhost:8765");
/// if client.is_available().await {
///     let results = client.search("process data", "src/", 10).await?;
/// }
/// ```
#[derive(Debug, Clone)]
pub struct EmbeddingClient {
    /// Base URL of the embedding service
    base_url: String,
    /// Request timeout (reserved for future HTTP client integration)
    _timeout: Duration,
}

impl Default for EmbeddingClient {
    fn default() -> Self {
        Self::new(DEFAULT_EMBEDDING_URL)
    }
}

impl EmbeddingClient {
    /// Create a new embedding client
    ///
    /// # Arguments
    /// * `base_url` - Base URL of the embedding service (e.g., "http://localhost:8765")
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            _timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Create a client with custom timeout
    pub fn with_timeout(base_url: &str, timeout: Duration) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            _timeout: timeout,
        }
    }

    /// Get the base URL
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Check if the embedding service is available
    ///
    /// # Returns
    /// `true` if the service responds to health check, `false` otherwise
    pub fn is_available(&self) -> bool {
        // Synchronous check - try to connect
        // This is a simplified check; in production, use async
        std::net::TcpStream::connect_timeout(&self.parse_address(), Duration::from_secs(1)).is_ok()
    }

    /// Parse the base URL into a socket address
    fn parse_address(&self) -> std::net::SocketAddr {
        let url = self
            .base_url
            .strip_prefix("http://")
            .unwrap_or(&self.base_url);
        let url = url.strip_prefix("https://").unwrap_or(url);

        // Default port
        let (host, port) = if let Some((h, p)) = url.split_once(':') {
            (h, p.parse().unwrap_or(8765))
        } else {
            (url, 8765)
        };

        // Resolve to socket address
        use std::net::ToSocketAddrs;
        format!("{}:{}", host, port)
            .to_socket_addrs()
            .ok()
            .and_then(|mut addrs| addrs.next())
            .unwrap_or_else(|| std::net::SocketAddr::from(([127, 0, 0, 1], port)))
    }

    /// Perform semantic search
    ///
    /// # Arguments
    /// * `query` - Search query text
    /// * `project` - Project path to search in
    /// * `top_k` - Number of results to return
    ///
    /// # Returns
    /// Vector of semantic search results, or error if service unavailable
    ///
    /// # Mitigation M8
    /// Returns ConnectionFailed error if service is unavailable,
    /// allowing caller to fall back to BM25-only search.
    pub fn search(
        &self,
        query: &str,
        project: &str,
        top_k: usize,
    ) -> TldrResult<Vec<SemanticResult>> {
        // For sync implementation, check availability first
        if !self.is_available() {
            return Err(TldrError::ConnectionFailed(format!(
                "Embedding service at {} is not available",
                self.base_url
            )));
        }

        // In a real implementation, this would make an HTTP request
        // For now, return empty results to allow compilation and testing
        // The actual HTTP request would use reqwest or similar

        // Placeholder: return empty results
        // Real implementation would POST to {base_url}/search
        let _request = SearchRequest {
            query: query.to_string(),
            top_k,
            project: project.to_string(),
        };

        // TODO: Implement actual HTTP request
        // For now, signal that service call would happen
        Ok(Vec::new())
    }

    /// Get embedding for a single text
    ///
    /// # Arguments
    /// * `text` - Text to embed
    ///
    /// # Returns
    /// Embedding vector of dimension EMBEDDING_DIM
    pub fn embed(&self, text: &str) -> TldrResult<Vec<f32>> {
        if !self.is_available() {
            return Err(TldrError::ConnectionFailed(format!(
                "Embedding service at {} is not available",
                self.base_url
            )));
        }

        let _request = EmbeddingRequest {
            text: text.to_string(),
            texts: None,
        };

        // TODO: Implement actual HTTP request
        // Return placeholder zeros for now
        Ok(vec![0.0; EMBEDDING_DIM])
    }

    /// Get embeddings for multiple texts
    ///
    /// # Arguments
    /// * `texts` - Texts to embed
    ///
    /// # Returns
    /// Vector of embedding vectors
    pub fn embed_batch(&self, texts: &[String]) -> TldrResult<Vec<Vec<f32>>> {
        if !self.is_available() {
            return Err(TldrError::ConnectionFailed(format!(
                "Embedding service at {} is not available",
                self.base_url
            )));
        }

        let _request = EmbeddingRequest {
            text: String::new(),
            texts: Some(texts.to_vec()),
        };

        // TODO: Implement actual HTTP request
        // Return placeholder zeros for now
        Ok(texts.iter().map(|_| vec![0.0; EMBEDDING_DIM]).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = EmbeddingClient::new("http://localhost:8765");
        assert_eq!(client.base_url(), "http://localhost:8765");
    }

    #[test]
    fn test_client_with_trailing_slash() {
        let client = EmbeddingClient::new("http://localhost:8765/");
        assert_eq!(client.base_url(), "http://localhost:8765");
    }

    #[test]
    fn test_client_unavailable() {
        // Use a port that's unlikely to be in use
        let client = EmbeddingClient::new("http://localhost:59999");
        assert!(!client.is_available());
    }

    #[test]
    fn test_search_unavailable_service() {
        let client = EmbeddingClient::new("http://localhost:59999");
        let result = client.search("query", "project", 10);
        assert!(result.is_err());

        if let Err(TldrError::ConnectionFailed(msg)) = result {
            assert!(msg.contains("not available"));
        } else {
            panic!("Expected ConnectionFailed error");
        }
    }
}
