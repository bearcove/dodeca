//! RPC protocol for dodeca CSS plugin
//!
//! Defines services for CSS URL rewriting and minification.

use facet::Facet;
use std::collections::HashMap;

/// Result of CSS processing operations
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum CssResult {
    /// Successfully processed CSS
    Success { css: String },
    /// Error during processing
    Error { message: String },
}

/// CSS processing service implemented by the plugin.
///
/// The host calls these methods to process CSS content.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait CssProcessor {
    /// Rewrite URLs in CSS and minify.
    ///
    /// Takes CSS source code and a path map for URL rewriting,
    /// returns processed and minified CSS.
    async fn rewrite_and_minify(&self, css: String, path_map: HashMap<String, String>) -> CssResult;
}