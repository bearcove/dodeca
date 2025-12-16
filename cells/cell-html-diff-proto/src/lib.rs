//! RPC protocol for dodeca HTML diff cell
//!
//! Defines services for HTML DOM diffing.

use facet::Facet;

// Re-export patch types from protocol
pub use dodeca_protocol::{NodePath, Patch};

/// Input for HTML diffing
#[derive(Debug, Clone, Facet)]
pub struct DiffInput {
    pub old_html: String,
    pub new_html: String,
}

/// Result of diffing two DOM trees
#[derive(Debug, Clone, Facet)]
pub struct DiffResult {
    /// Patches to apply (in order)
    pub patches: Vec<Patch>,
    /// Stats for debugging
    pub nodes_compared: usize,
    pub nodes_skipped: usize,
}

/// Result of HTML diff operations
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum HtmlDiffResult {
    /// Successfully diffed HTML
    Success { result: DiffResult },
    /// Error during diffing
    Error { message: String },
}

/// HTML diff service implemented by the cell.
///
/// The host calls these methods to diff HTML documents.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait HtmlDiffer {
    /// Diff two HTML documents and produce patches to transform old into new
    async fn diff_html(&self, input: DiffInput) -> HtmlDiffResult;
}
