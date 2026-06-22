//! Typed interface for the dodeca text-embedding processor.
//!
//! Turns text into fixed-size vectors for semantic search. The v1 implementation
//! is a Model2Vec static embedder (CPU, no model forward pass); the interface is
//! deliberately model-agnostic so a heavier embedder (e.g. EmbeddingGemma via
//! CoreML) can drop in behind it later.

use facet::Facet;

/// Result of an [`Embedder::embed`] call.
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum EmbedResult {
    /// One L2-normalized vector per input text, in order. `dim` is their length.
    Success { vectors: Vec<Vec<f32>>, dim: u32 },
    /// Embedding failed (model load or tokenization error). `message` is safe to show.
    Error { message: String },
}

/// Text embedder. Dodeca calls this to embed page chunks and search queries into
/// the same vector space (cosine similarity = dot product, since outputs are
/// unit-normalized).
#[allow(async_fn_in_trait)]
pub trait Embedder {
    /// Embed each text into a unit-length vector. The output order matches the
    /// input order.
    async fn embed(&self, texts: Vec<String>) -> EmbedResult;
}
