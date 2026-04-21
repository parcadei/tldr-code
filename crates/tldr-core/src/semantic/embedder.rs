//! Embedding service using fastembed-rs
//!
//! This module provides the `Embedder` struct for generating dense embeddings
//! from text using the Snowflake Arctic model family. It wraps fastembed-rs
//! to provide a type-safe, validated embedding service.
//!
//! # Architecture
//!
//! The Embedder handles:
//! - Model loading with progress reporting
//! - Model integrity validation (P0 mitigation)
//! - Single text and batch embedding
//! - Automatic normalization of output vectors
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::semantic::{Embedder, EmbeddingModel};
//!
//! // Create embedder with default model (Arctic-M)
//! let embedder = Embedder::new(EmbeddingModel::default())?;
//!
//! // Embed a single text
//! let embedding = embedder.embed_text("fn process_data() { }")?;
//! assert_eq!(embedding.len(), 768); // Arctic-M dimensions
//!
//! // Batch embedding
//! let texts = vec!["fn foo() {}", "fn bar() {}"];
//! let embeddings = embedder.embed_batch(texts, false)?;
//! assert_eq!(embeddings.len(), 2);
//! ```
//!
//! # P0 Mitigations (from premortem)
//!
//! - **1.1**: Validates ONNX runtime before model load
//! - **1.3**: Shows progress message before model download
//! - **4.1**: Model integrity validation after load (dimension check)

use fastembed::{EmbeddingModel as FastEmbeddingModel, InitOptions, TextEmbedding};

use crate::error::TldrError;
use crate::semantic::similarity::normalize;
use crate::semantic::types::EmbeddingModel;
use crate::TldrResult;

/// Options for embedding operations
///
/// Controls embedding behavior such as progress display and query prefixes.
#[derive(Debug, Clone, Default)]
pub struct EmbedOptions {
    /// Model to use (default: ArcticM)
    pub model: EmbeddingModel,

    /// Show progress during embedding
    pub show_progress: bool,

    /// Use query:/passage: prefixes for Arctic models (P1 mitigation 5.4)
    ///
    /// Arctic models perform better when queries use "query: " prefix
    /// and documents use "passage: " prefix. Enable this for search queries.
    pub use_prefix: bool,
}

/// Embedding service wrapping fastembed-rs
///
/// Provides validated embedding generation with automatic normalization.
/// The embedder performs model integrity checks on initialization to
/// detect corrupted model files early.
///
/// # Thread Safety
///
/// `Embedder` is `Send` but not `Sync` - create one per thread for
/// concurrent embedding.
pub struct Embedder {
    /// The underlying fastembed TextEmbedding instance
    model: TextEmbedding,

    /// Configuration for this embedder
    config: EmbeddingModel,
}

impl Embedder {
    /// Create a new embedder with the specified model
    ///
    /// # Arguments
    ///
    /// * `model` - The embedding model variant to use
    ///
    /// # Returns
    ///
    /// * `TldrResult<Self>` - Initialized embedder or error
    ///
    /// # Errors
    ///
    /// * `TldrError::ModelLoadError` - ONNX runtime unavailable or model download failed
    /// * `TldrError::Embedding` - Model integrity check failed
    ///
    /// # P0 Mitigations
    ///
    /// - Shows progress message before download (1.3)
    /// - Validates ONNX runtime (1.1)
    /// - Checks model integrity after load (4.1)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let embedder = Embedder::new(EmbeddingModel::ArcticM)?;
    /// ```
    pub fn new(model: EmbeddingModel) -> TldrResult<Self> {
        // Convert our model enum to fastembed's
        let fast_model = Self::to_fastembed_model(model);

        // P0 Mitigation 1.3: Progress message before download
        eprintln!(
            "Loading embedding model ({})... First run may download ~{}MB model.",
            model.model_name(),
            Self::model_size_mb(model)
        );

        // Initialize the model
        // P0 Mitigation 1.1: fastembed will fail here if ONNX runtime is unavailable
        let mut embedding = TextEmbedding::try_new(InitOptions::new(fast_model)).map_err(|e| {
            TldrError::ModelLoadError {
                model: model.model_name().to_string(),
                detail: e.to_string(),
            }
        })?;

        // P0 Mitigation 4.1: Model integrity check
        // Embed a known input and verify dimensions
        let test_result = embedding
            .embed(vec!["test"], None)
            .map_err(|e| TldrError::Embedding(format!("Model integrity check failed: {}", e)))?;

        if test_result.is_empty() {
            return Err(TldrError::Embedding(
                "Model integrity check failed: empty result".to_string(),
            ));
        }

        let actual_dims = test_result[0].len();
        let expected_dims = model.dimensions();

        if actual_dims != expected_dims {
            return Err(TldrError::Embedding(format!(
                "Model integrity check failed: expected {} dimensions, got {}",
                expected_dims, actual_dims
            )));
        }

        Ok(Self {
            model: embedding,
            config: model,
        })
    }

    /// Convert our EmbeddingModel to fastembed's enum
    fn to_fastembed_model(model: EmbeddingModel) -> FastEmbeddingModel {
        match model {
            EmbeddingModel::ArcticXS => FastEmbeddingModel::SnowflakeArcticEmbedXS,
            EmbeddingModel::ArcticS => FastEmbeddingModel::SnowflakeArcticEmbedS,
            EmbeddingModel::ArcticM => FastEmbeddingModel::SnowflakeArcticEmbedM,
            EmbeddingModel::ArcticMLong => FastEmbeddingModel::SnowflakeArcticEmbedMLong,
            EmbeddingModel::ArcticL => FastEmbeddingModel::SnowflakeArcticEmbedL,
        }
    }

    /// Get approximate model size in MB for progress messages
    fn model_size_mb(model: EmbeddingModel) -> usize {
        match model {
            EmbeddingModel::ArcticXS => 30,
            EmbeddingModel::ArcticS => 90,
            EmbeddingModel::ArcticM | EmbeddingModel::ArcticMLong => 110,
            EmbeddingModel::ArcticL => 335,
        }
    }

    /// Embed a single text string
    ///
    /// Returns a normalized embedding vector with L2 norm = 1.0.
    ///
    /// # Arguments
    ///
    /// * `text` - Text to embed
    ///
    /// # Returns
    ///
    /// * `TldrResult<Vec<f32>>` - Normalized embedding vector
    ///
    /// # Invariants
    ///
    /// * Output length == model.dimensions()
    /// * Output is normalized (L2 norm == 1.0)
    /// * Empty input returns zero vector
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let embedding = embedder.embed_text("fn process_data() { }")?;
    /// assert_eq!(embedding.len(), embedder.config().dimensions());
    /// ```
    pub fn embed_text(&mut self, text: &str) -> TldrResult<Vec<f32>> {
        // Handle empty input - return zero vector
        if text.is_empty() {
            return Ok(vec![0.0; self.config.dimensions()]);
        }

        let result = self
            .model
            .embed(vec![text], None)
            .map_err(|e| TldrError::Embedding(format!("Failed to embed text: {}", e)))?;

        let mut embedding = result
            .into_iter()
            .next()
            .ok_or_else(|| TldrError::Embedding("No embedding returned".to_string()))?;

        // Normalize to unit length
        normalize(&mut embedding);

        Ok(embedding)
    }

    /// Embed multiple texts in a batch
    ///
    /// More efficient than calling `embed_text` multiple times as it batches
    /// the model inference.
    ///
    /// # Arguments
    ///
    /// * `texts` - Texts to embed
    /// * `show_progress` - Whether to show progress (uses batch_size for chunking)
    ///
    /// # Returns
    ///
    /// * `TldrResult<Vec<Vec<f32>>>` - Normalized embedding vectors
    ///
    /// # Performance
    ///
    /// * Batching reduces overhead for multiple texts
    /// * Default batch size: 32
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let texts = vec!["fn foo() {}", "fn bar() {}"];
    /// let embeddings = embedder.embed_batch(texts, false)?;
    /// assert_eq!(embeddings.len(), 2);
    /// ```
    pub fn embed_batch(
        &mut self,
        texts: Vec<&str>,
        show_progress: bool,
    ) -> TldrResult<Vec<Vec<f32>>> {
        // Handle empty input
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Use batch size for progress (affects how fastembed chunks the work)
        let batch_size = if show_progress { Some(32) } else { None };

        let results = self
            .model
            .embed(texts, batch_size)
            .map_err(|e| TldrError::Embedding(format!("Failed to embed batch: {}", e)))?;

        // Normalize all embeddings
        let normalized: Vec<Vec<f32>> = results
            .into_iter()
            .map(|mut v| {
                normalize(&mut v);
                v
            })
            .collect();

        Ok(normalized)
    }

    /// Get the model configuration
    ///
    /// Returns the `EmbeddingModel` variant this embedder was created with.
    pub fn config(&self) -> EmbeddingModel {
        self.config
    }

    /// Get embedding dimensions for this model
    ///
    /// Convenience method that delegates to `config().dimensions()`.
    pub fn dimensions(&self) -> usize {
        self.config.dimensions()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::similarity::is_normalized;

    // =========================================================================
    // All embedding tests are #[ignore] by default since they require
    // model download (~110MB for Arctic-M). Run with:
    //   cargo test --release -p tldr-core -- --ignored embedder
    // =========================================================================

    #[test]
    fn embed_options_default_values() {
        // GIVEN/WHEN: Default EmbedOptions
        let options = EmbedOptions::default();

        // THEN: Should have sensible defaults
        assert_eq!(options.model, EmbeddingModel::ArcticM);
        assert!(!options.show_progress);
        assert!(!options.use_prefix);
    }

    #[test]
    #[ignore = "Requires model download (~110MB)"]
    fn embedder_new_initializes_model() {
        // GIVEN: A model variant
        let model = EmbeddingModel::ArcticM;

        // WHEN: We create an embedder
        let embedder = Embedder::new(model);

        // THEN: Should succeed
        assert!(
            embedder.is_ok(),
            "Failed to initialize: {:?}",
            embedder.err()
        );

        let embedder = embedder.unwrap();
        assert_eq!(embedder.config(), model);
        assert_eq!(embedder.dimensions(), 768);
    }

    #[test]
    #[ignore = "Requires model download (~110MB)"]
    fn embedder_embed_text_returns_correct_dimensions() {
        // GIVEN: An initialized embedder
        let mut embedder = Embedder::new(EmbeddingModel::ArcticM).expect("Failed to init");

        // WHEN: We embed text
        let embedding = embedder
            .embed_text("fn process_data() { }")
            .expect("Failed to embed");

        // THEN: Should have correct dimensions
        assert_eq!(embedding.len(), 768, "Expected 768 dimensions for ArcticM");
    }

    #[test]
    #[ignore = "Requires model download (~110MB)"]
    fn embedder_embed_text_is_normalized() {
        // GIVEN: An initialized embedder
        let mut embedder = Embedder::new(EmbeddingModel::ArcticM).expect("Failed to init");

        // WHEN: We embed text
        let embedding = embedder
            .embed_text("fn process_data() { }")
            .expect("Failed to embed");

        // THEN: Embedding should be normalized (L2 norm = 1.0)
        assert!(
            is_normalized(&embedding),
            "Embedding should have L2 norm = 1.0"
        );
    }

    #[test]
    #[ignore = "Requires model download (~110MB)"]
    fn embedder_batch_embedding_matches_single() {
        // GIVEN: An initialized embedder and some texts
        let mut embedder = Embedder::new(EmbeddingModel::ArcticM).expect("Failed to init");
        let text1 = "fn foo() { }";
        let text2 = "fn bar() { }";

        // WHEN: We embed individually and in batch
        let single1 = embedder.embed_text(text1).expect("Failed single embed 1");
        let single2 = embedder.embed_text(text2).expect("Failed single embed 2");
        let batch = embedder
            .embed_batch(vec![text1, text2], false)
            .expect("Failed batch embed");

        // THEN: Results should match (within floating point tolerance)
        assert_eq!(batch.len(), 2);

        // Compare with tolerance for floating point differences
        for (a, b) in single1.iter().zip(batch[0].iter()) {
            assert!(
                (a - b).abs() < 1e-5,
                "Single vs batch mismatch: {} vs {}",
                a,
                b
            );
        }
        for (a, b) in single2.iter().zip(batch[1].iter()) {
            assert!(
                (a - b).abs() < 1e-5,
                "Single vs batch mismatch: {} vs {}",
                a,
                b
            );
        }
    }

    #[test]
    #[ignore = "Requires model download (~110MB)"]
    fn embedder_empty_input_returns_zero_vector() {
        // GIVEN: An initialized embedder
        let mut embedder = Embedder::new(EmbeddingModel::ArcticM).expect("Failed to init");

        // WHEN: We embed empty string
        let embedding = embedder.embed_text("").expect("Failed to embed empty");

        // THEN: Should return zero vector with correct dimensions
        assert_eq!(embedding.len(), 768);
        assert!(
            embedding.iter().all(|&x| x == 0.0),
            "Empty input should produce zero vector"
        );
    }

    #[test]
    #[ignore = "Requires model download (~110MB)"]
    fn embedder_batch_empty_list_returns_empty() {
        // GIVEN: An initialized embedder
        let mut embedder = Embedder::new(EmbeddingModel::ArcticM).expect("Failed to init");

        // WHEN: We embed empty list
        let embeddings = embedder
            .embed_batch(vec![], false)
            .expect("Failed to embed empty batch");

        // THEN: Should return empty list
        assert!(embeddings.is_empty());
    }

    #[test]
    #[ignore = "Requires model download (~30MB for XS)"]
    fn embedder_xs_model_dimensions() {
        // GIVEN: Arctic XS model (smallest, fastest for testing)
        let mut embedder = Embedder::new(EmbeddingModel::ArcticXS).expect("Failed to init XS");

        // WHEN: We embed text
        let embedding = embedder.embed_text("test").expect("Failed to embed");

        // THEN: Should have 384 dimensions
        assert_eq!(embedding.len(), 384);
        assert!(is_normalized(&embedding));
    }

    #[test]
    #[ignore = "Requires model download (~110MB)"]
    fn embedder_deterministic_results() {
        // GIVEN: An initialized embedder
        let mut embedder = Embedder::new(EmbeddingModel::ArcticM).expect("Failed to init");
        let text = "fn process_data(input: &str) -> Result<Output>";

        // WHEN: We embed the same text twice
        let e1 = embedder.embed_text(text).expect("Failed embed 1");
        let e2 = embedder.embed_text(text).expect("Failed embed 2");

        // THEN: Results should be identical
        assert_eq!(e1.len(), e2.len());
        for (a, b) in e1.iter().zip(e2.iter()) {
            assert!(
                (a - b).abs() < 1e-6,
                "Embeddings should be deterministic: {} vs {}",
                a,
                b
            );
        }
    }
}
