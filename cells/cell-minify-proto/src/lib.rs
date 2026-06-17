//! Typed interface for dodeca minify processor
//!
//! Defines services for HTML minification.

use facet::Facet;

/// Result of minification operations
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum MinifyResult {
    /// Successfully minified content
    Success { content: String },
    /// Error during minification
    Error { message: String },
}

/// Minifier interface.
///
/// Dodeca calls these methods to minify content.
#[allow(async_fn_in_trait)]
pub trait Minifier {
    /// Minify HTML content
    ///
    /// Returns minified HTML, or an error if minification fails.
    async fn minify_html(&self, html: String) -> MinifyResult;
}
