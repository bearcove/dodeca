//! RPC protocol for dodeca HTML diff cell
//!
//! Defines services for HTML DOM diffing.

use facet::Facet;

/// Input for HTML diffing
#[derive(Debug, Clone, Facet)]
pub struct DiffInput {
    pub old_html: String,
    pub new_html: String,
}

/// Result of diffing two DOM trees
#[derive(Debug, Clone, Facet)]
pub struct DiffOutcome {
    /// Patches to apply, as an opaque postcard blob (deserialized in the browser)
    pub patches_blob: Vec<u8>,
}

/// Error while diffing
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum DiffError {
    /// A generic error occured
    Generic(String),
}

/// HTML diff service implemented by the cell.
///
/// The host calls these methods to diff HTML documents.
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait HtmlDiffer {
    /// Diff two HTML documents and produce patches to transform old into new
    async fn diff_html(&self, input: DiffInput) -> Result<DiffOutcome, DiffError>;
}
